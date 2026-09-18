import time

from aqt import gui_hooks, mw
from aqt.utils import tooltip

from .client import (
    MAX_PENDING,
    PENDING_SQL,
    UNDO_WINDOW_MS,
    Client,
    describe,
    reconcile,
    rows_to_reviews,
    snapshot,
)

MARK_KEY = "ankiquestUploadedThrough"
RECENT_KEY = "ankiquestRecentUploads"
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
    window_start = int(time.time() * 1000) - UNDO_WINDOW_MS
    recent = {i for i in mw.pm.profile.get(RECENT_KEY, []) if i > window_start}

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
            profile = api.upload(
                rows_to_reviews(sent), rollover, mark == 0 or full, deleted if first else ()
            )
            first = False
            if not full:
                return max(known, mark), sorted(present), profile

    def done(future):
        state["busy"] = False
        try:
            mw.pm.profile[MARK_KEY], mw.pm.profile[RECENT_KEY], profile = future.result()
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


def on_operation(changes, handler):
    if handler is mw.reviewer:
        return
    if getattr(getattr(changes, "changes", changes), "study_queues", False):
        refresh(mw.state == "review")


def on_profile_open():
    state["previous"] = None
    refresh(False, resync=True)


gui_hooks.reviewer_did_answer_card.append(lambda *_: refresh(True))
gui_hooks.operation_did_execute.append(on_operation)
gui_hooks.sync_did_finish.append(lambda: refresh(False, resync=True))
gui_hooks.profile_did_open.append(on_profile_open)
