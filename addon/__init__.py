import time

from aqt import gui_hooks, mw
from aqt.qt import QAction
from aqt.utils import openLink, tooltip

from . import board, notify, ui
from .client import (
    MAX_PENDING,
    PENDING_SQL,
    UNDO_WINDOW_MS,
    Client,
    announcements,
    describe,
    offset_west_min,
    reconcile,
    rows_to_reviews,
    snapshot,
)
from .deck_completion import deck_snapshots, study_day

MARK_KEY = "ankiquestUploadedThrough"
RECENT_KEY = "ankiquestRecentUploads"
BASELINE_KEY = "ankiquestBaselineAccount"
SHARED_KEY = "ankiquestSharedDecks"
ORDER_KEY = "ankiquestLastOrder"
INBOX_KEY = "ankiquestInboxCursor"
STREAK_DAY_KEY = "ankiquestStreakDay"
RESYNC_WINDOW_MS = 7 * 86_400_000
POLL_MS = 5 * 60 * 1000

state = {
    "previous": None,
    "busy": False,
    "again": False,
    "polling": False,
    "timer": None,
    "board": None,
    "profile": None,
    "inbox": [],
}


def config():
    return mw.addonManager.getConfig(__name__) or {}


def save_config(values):
    current = config()
    current.update(values)
    mw.addonManager.writeConfig(__name__, current)


def client():
    settings = config()
    return Client(settings.get("url", ""), settings.get("user", ""), settings.get("token", ""))


def refresh(show_feedback, resync=False):
    if mw.col is None:
        return
    if state["busy"]:
        state["again"] = True
        return
    api = client()
    if not api.configured:
        return
    state["busy"] = True
    rollover = int(mw.col.get_config("rollover", 4))
    mark = int(mw.pm.profile.get(MARK_KEY, 0))
    account = [api.base, api.user]
    initial_sync = mark == 0 and mw.pm.profile.get(BASELINE_KEY) != account
    start = max(0, mark - RESYNC_WINDOW_MS) if resync else mark
    window_start = int(time.time() * 1000) - UNDO_WINDOW_MS
    recent = {i for i in mw.pm.profile.get(RECENT_KEY, []) if i > window_start}
    shared = set(mw.pm.profile.get(SHARED_KEY, []))

    def work():
        window = mw.col.db.all(PENDING_SQL, window_start)
        present, deleted, restored = reconcile(window, recent, start, mark)
        known = start
        first = True
        while True:
            rows = mw.col.db.all(PENDING_SQL, known)
            full = len(rows) == MAX_PENDING
            if rows:
                known = rows[-1][0]
            sent = rows + restored if first else rows
            decks = None
            clock_offset = offset_west_min()
            if not full and shared:
                try:
                    now_ms = int(time.time() * 1000)
                    decks = deck_snapshots(mw.col, now_ms, clock_offset, rollover, only=shared)
                    # Do not send mixed-day counts if a rollover occurred while
                    # reading them. The next refresh will collect the new day.
                    if study_day(int(time.time() * 1000), clock_offset, rollover) != study_day(
                        now_ms, clock_offset, rollover
                    ):
                        decks = None
                except Exception as e:
                    print("ankiquest deck progress:", e)
            profile = api.upload(
                rows_to_reviews(sent), rollover, initial_sync or full, deleted if first else (),
                decks=decks, clock_offset=clock_offset,
            )
            first = False
            if not full:
                return max(known, mark), sorted(present), profile

    def done(future):
        state["busy"] = False
        try:
            mw.pm.profile[MARK_KEY], mw.pm.profile[RECENT_KEY], profile = future.result()
            mw.pm.profile[BASELINE_KEY] = account
        except Exception as e:
            print("ankiquest:", e)
            return
        after = snapshot(profile)
        before, state["previous"] = state["previous"], after
        told = announcements(profile)
        message = describe(before, after) if before else None
        lines = list(told)
        if message and (show_feedback or told):
            lines.append(message[0])
        if lines:
            tooltip("<br>".join(lines), period=3500 if told or (message and message[1]) else 1800)
        if state["again"]:
            state["again"] = False
            refresh(True)

    mw.taskman.run_in_background(work, done, uses_collection=True)


def refresh_shared_decks():
    api = client()
    if not api.configured:
        return

    def done(future):
        try:
            mw.pm.profile[SHARED_KEY] = future.result()
        except Exception as e:
            print("ankiquest shared decks:", e)

    mw.taskman.run_in_background(api.shared_decks, done)


def poll(quiet=False):
    """Keeps the deck list board, the inbox and the streak warning up to date."""
    api = client()
    if not api.configured or state["polling"]:
        return
    state["polling"] = True

    def work():
        return api.leaderboard(), api.profile(), api.notifications()

    def done(future):
        state["polling"] = False
        try:
            standings, profile, inbox = future.result()
        except Exception as e:
            print("ankiquest poll:", e)
            return
        state["board"], state["profile"], state["inbox"] = standings, profile, inbox
        announce(api, standings, profile, inbox, quiet)
        redraw()

    mw.taskman.run_in_background(work, done)


def announce(api, standings, profile, inbox, quiet):
    settings = config()
    now_ms = int(time.time() * 1000)
    previous = mw.pm.profile.get(ORDER_KEY)
    mw.pm.profile[ORDER_KEY] = [row["user"] for row in standings]
    fresh, cursor = notify.fresh_messages(inbox, mw.pm.profile.get(INBOX_KEY, 0), now_ms // 1000)
    mw.pm.profile[INBOX_KEY] = cursor
    rollover = int(mw.col.get_config("rollover", 4)) if mw.col else 4
    warning = notify.streak_message(
        profile,
        int(settings.get("streak_hours", 2) or 0),
        now_ms,
        offset_west_min(),
        rollover,
        mw.pm.profile.get(STREAK_DAY_KEY),
    )
    if warning:
        mw.pm.profile[STREAK_DAY_KEY] = warning[1]
    if quiet:
        return

    if settings.get("notify_rank", True):
        change = notify.rank_message(previous, standings, api.name)
        if change:
            tooltip(change, period=4000)
    if warning:
        tooltip(warning[0], period=5000)
    for entry in fresh:
        text = notify.summarize(entry).replace("\n", "<br>")
        if notify.answerable(entry):
            text += "<br><i>Tools → ankiquest inbox to reply</i>"
        tooltip(text, period=6000)


def redraw():
    if mw.state == "deckBrowser":
        mw.deckBrowser.refresh()


def on_deck_browser(deck_browser, content):
    api = client()
    if not api.configured:
        return
    if state["board"] is None and not state["polling"]:
        poll(quiet=True)
    waiting = sum(1 for entry in state["inbox"] if notify.answerable(entry))
    content.stats += board.html(
        state["board"] or [],
        config().get("period", board.DEFAULT_PERIOD),
        api.name,
        state["profile"],
        waiting,
    )


def on_js_message(handled, message, context):
    if message.startswith("ankiquest:period:"):
        period = message.split(":", 2)[2]
        if period in dict(board.PERIODS):
            save_config({"period": period})
            redraw()
        return (True, None)
    if message == "ankiquest:inbox":
        open_inbox()
        return (True, None)
    return handled


def send_reply(entry, message, status):
    api = client()

    def work():
        return api.reply(entry["id"], message)

    def done(future):
        try:
            who = future.result()
        except Exception as e:
            _say(status, "Could not send: %s" % e)
            return
        entry["replied"] = True
        _say(status, "Sent to %s" % who)
        redraw()

    mw.taskman.run_in_background(work, done)


def _say(status, text):
    try:
        status.setText(text)
    except RuntimeError:
        tooltip(text)


def open_inbox():
    api = client()
    if not api.configured:
        tooltip("Set the server, player and token in ankiquest settings first.")
        return

    def done(future):
        try:
            entries = future.result()
        except Exception as e:
            tooltip("ankiquest: could not load your inbox (%s)" % e)
            return
        state["inbox"] = entries
        ids = [entry["id"] for entry in entries] + [mw.pm.profile.get(INBOX_KEY, 0)]
        mw.pm.profile[INBOX_KEY] = max(ids)
        ui.inbox_dialog(mw, entries, send_reply)
        redraw()

    mw.taskman.run_in_background(api.notifications, done)


def open_settings():
    values = ui.settings_dialog(mw, config(), test_connection, upload_everything)
    if values is None:
        return
    save_config(values)
    state["previous"] = None
    state["board"] = None
    refresh_shared_decks()
    poll(quiet=True)
    tooltip("ankiquest settings saved.")


def test_connection(values):
    api = Client(values["url"], values["user"], values["token"])
    if not api.configured:
        tooltip("Fill in the server, player and token first.")
        return

    def done(future):
        try:
            profile = future.result()
        except Exception as e:
            tooltip("ankiquest: %s" % e)
            return
        tooltip("Connected as %s, level %d." % (values["user"], profile.get("level", 1)))

    mw.taskman.run_in_background(api.profile, done)


def upload_everything():
    mw.pm.profile[MARK_KEY] = 0
    mw.pm.profile[RECENT_KEY] = []
    state["previous"] = None
    refresh(False, resync=True)
    tooltip("Uploading your whole review history…")


def open_deck_notifications():
    api = client()
    if mw.col is None or not api.configured:
        tooltip("Set the server, player and token in ankiquest settings first.")
        return
    rollover = int(mw.col.get_config("rollover", 4))

    def work():
        clock_offset = offset_west_min()
        catalog = deck_snapshots(mw.col, int(time.time() * 1000), clock_offset, rollover)
        api.upload([], rollover, True, decks=catalog, clock_offset=clock_offset, catalog=True)
        return api.deck_settings()

    def done(future):
        try:
            settings = future.result()
        except Exception as e:
            tooltip("ankiquest: could not load your decks (%s)" % e)
            return
        choice = ui.deck_dialog(mw, settings)
        if choice is None:
            return
        shared, unshared, recipients, nudges = choice
        if shared and not recipients:
            tooltip("Pick at least one person to notify, or share no decks.")
            return
        save_deck_choice(api, shared, unshared, recipients, nudges)

    mw.taskman.run_in_background(work, done, uses_collection=True)


def save_deck_choice(api, shared, unshared, recipients, nudges):
    def work():
        api.save_deck_settings(shared, unshared, recipients, nudges)
        return api.shared_decks()

    def done(future):
        try:
            mw.pm.profile[SHARED_KEY] = future.result()
        except Exception as e:
            tooltip("ankiquest: could not save your decks (%s)" % e)
            return
        tooltip("%d deck%s shared." % (len(shared), "" if len(shared) == 1 else "s"))

    mw.taskman.run_in_background(work, done)


def open_website():
    api = client()
    if not api.base:
        tooltip("Set the server in ankiquest settings first.")
        return
    openLink("%s/#%s" % (api.base, api.user))


def on_operation(changes, handler):
    if handler is mw.reviewer:
        return
    if getattr(getattr(changes, "changes", changes), "study_queues", False):
        refresh(mw.state == "review")


def on_profile_open():
    state["previous"] = None
    if state["timer"] is None:
        try:
            state["timer"] = mw.progress.timer(POLL_MS, poll, repeat=True, parent=mw)
        except TypeError:
            state["timer"] = mw.progress.timer(POLL_MS, poll, True)
    refresh_shared_decks()
    refresh(False, resync=True)
    poll(quiet=True)


def on_sync():
    refresh_shared_decks()
    refresh(False, resync=True)
    poll()


def add_action(title, handler):
    action = QAction(title, mw)
    action.triggered.connect(handler)
    mw.form.menuTools.addAction(action)


add_action("ankiquest settings…", open_settings)
add_action("ankiquest deck notifications…", open_deck_notifications)
add_action("ankiquest inbox…", open_inbox)
add_action("ankiquest on the web…", open_website)


gui_hooks.reviewer_did_answer_card.append(lambda *_: refresh(True))
gui_hooks.operation_did_execute.append(on_operation)
gui_hooks.sync_did_finish.append(on_sync)
gui_hooks.profile_did_open.append(on_profile_open)
gui_hooks.deck_browser_will_render_content.append(on_deck_browser)
gui_hooks.webview_did_receive_js_message.append(on_js_message)
