from aqt import gui_hooks, mw
from aqt.utils import tooltip

from .client import MAX_PENDING, PENDING_SQL, Client, describe, rows_to_reviews, snapshot

MARK_KEY = "ankiquestUploadedThrough"
RESYNC_WINDOW_MS = 7 * 86_400_000

state = {"previous": None, "busy": False, "again": False}


def client():
    config = mw.addonManager.getConfig(__name__) or {}
    return Client(config.get("url", ""), config.get("user", ""), config.get("token", ""))


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
    start = max(0, mark - RESYNC_WINDOW_MS) if resync else mark

    def work():
        known = start
        while True:
            rows = mw.col.db.all(PENDING_SQL, known)
            full = len(rows) == MAX_PENDING
            profile = api.upload(rows_to_reviews(rows), rollover, mark == 0 or full)
            if rows:
                known = rows[-1][0]
            if not full:
                return max(known, mark), profile

    def done(future):
        state["busy"] = False
        try:
            mw.pm.profile[MARK_KEY], profile = future.result()
        except Exception as e:
            print("ankiquest:", e)
            return
        after = snapshot(profile)
        before, state["previous"] = state["previous"], after
        message = describe(before, after) if before else None
        if show_feedback and message:
            text, important = message
            tooltip(text, period=3500 if important else 1800)
        if state["again"]:
            state["again"] = False
            refresh(True)

    mw.taskman.run_in_background(work, done)


def on_profile_open():
    state["previous"] = None
    refresh(False, resync=True)


gui_hooks.reviewer_did_answer_card.append(lambda *_: refresh(True))
gui_hooks.sync_did_finish.append(lambda: refresh(False, resync=True))
gui_hooks.profile_did_open.append(on_profile_open)
