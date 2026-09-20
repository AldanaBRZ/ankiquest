"""Desktop leaderboard and notification wording; run with python -m unittest discover -s tests."""

import importlib.util
import sys
import unittest
from pathlib import Path

ADDON = Path(__file__).resolve().parents[1] / "addon"


def load_module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


board = load_module("ankiquest_board", ADDON / "board.py")
notify = load_module("ankiquest_notify", ADDON / "notify.py")


def player(user, week, level=1, streak=0, **periods):
    row = {
        "user": user,
        "display": user.title(),
        "level": level,
        "streak": streak,
        "week_xp": week,
    }
    if periods:
        row["periods"] = dict(periods, week=week)
    return row


class BoardTests(unittest.TestCase):
    def test_each_period_ranks_by_its_own_experience(self):
        rows = [player("slow", 10, day=99), player("fast", 500, day=1)]
        self.assertEqual(["fast", "slow"], [r["user"] for r in board.ranked(rows, "week")])
        self.assertEqual(["slow", "fast"], [r["user"] for r in board.ranked(rows, "day")])

    def test_a_board_without_periods_still_reads_the_week(self):
        rows = [player("cerro", 40), player("hill", 90)]
        self.assertEqual(["hill", "cerro"], [r["user"] for r in board.ranked(rows, "month")])
        self.assertEqual(90, board.xp_of(rows[1], "month"))

    def test_the_leader_wears_the_crown_and_i_am_bold(self):
        html = board.html([player("cerro", 40), player("hill", 90, streak=3)], "week", "cerro")
        self.assertIn("\U0001f451", html)
        self.assertIn("font-weight:bold'><td", html.replace("\n", ""))
        self.assertIn("\U0001f525 3", html)
        self.assertLess(html.index("Hill"), html.index("Cerro"))

    def test_the_chosen_period_is_the_one_marked(self):
        html = board.html([player("cerro", 40)], "month", "cerro")
        self.assertIn("pycmd('ankiquest:period:month')", html)
        self.assertIn("font-weight:bold;text-decoration:underline;margin-left:.6em'", html)
        self.assertIn("<span style='opacity:.7'>Month</span>", html)
        self.assertEqual(1, html.count("text-decoration:underline"))

    def test_an_empty_board_and_a_waiting_inbox_are_both_visible(self):
        html = board.html([], "week", "cerro", {"level": 4, "xp_into_level": 1, "xp_for_next": 2}, 2)
        self.assertIn("No one has any experience yet.", html)
        self.assertIn("pycmd('ankiquest:inbox')", html)
        self.assertIn("Lv 4", html)

    def test_names_cannot_smuggle_markup_into_the_deck_list(self):
        html = board.html([{"user": "x", "display": "<script>", "level": 1, "streak": 0, "week_xp": 1}], "week", "x")
        self.assertNotIn("<script>", html)
        self.assertIn("&lt;script&gt;", html)


class NotifyTests(unittest.TestCase):
    def setUp(self):
        self.standings = [player("hill", 900), player("cerro", 500), player("mago", 100)]

    def test_taking_the_crown_and_being_passed_read_differently(self):
        climbed = notify.rank_message(["hill", "mago", "cerro"], self.standings, "cerro")
        self.assertIn("You're now #2", climbed)
        self.assertIn("You passed Mago.", climbed)
        self.assertIn("400 XP behind Hill.", climbed)

        crowned = notify.rank_message(["cerro", "hill", "mago"], self.standings, "hill")
        self.assertIn("\U0001f451 You took the crown", crowned)

        lost = notify.rank_message(["cerro", "hill", "mago"], self.standings, "cerro")
        self.assertIn("\U0001f451 Hill took the crown", lost)
        self.assertIn("You're now #2.", lost)

    def test_silence_without_movement_or_without_experience(self):
        order = ["hill", "cerro", "mago"]
        self.assertIsNone(notify.rank_message(order, self.standings, "cerro"))
        self.assertIsNone(notify.rank_message(None, self.standings, "cerro"))
        self.assertIsNone(notify.rank_message(order, self.standings, "nobody"))
        quiet = [player("hill", 0), player("cerro", 0)]
        self.assertIsNone(notify.rank_message(["cerro", "hill"], quiet, "hill"))

    def test_the_streak_warning_waits_for_the_last_hours_and_speaks_once(self):
        day = 20_000
        profile = {"at_risk": True, "streak": 7, "freezes": 0}
        early = day * notify.DAY_MS + 4 * notify.HOUR_MS
        self.assertIsNone(notify.streak_message(profile, 2, early, 0, 0, None))

        late = (day + 1) * notify.DAY_MS - notify.HOUR_MS
        text, marked = notify.streak_message(profile, 2, late, 0, 0, None)
        self.assertIn("7 day streak ends in 1h", text)
        self.assertIn("No freezes left.", text)
        self.assertEqual(day, marked)
        self.assertIsNone(notify.streak_message(profile, 2, late, 0, 0, marked))
        self.assertIsNone(notify.streak_message(profile, 0, late, 0, 0, None))
        self.assertIsNone(notify.streak_message({"at_risk": False}, 2, late, 0, 0, None))

        spare = dict(profile, freezes=1)
        self.assertIn("A freeze would cover you", notify.streak_message(spare, 2, late, 0, 0, None)[0])

    def test_only_recent_arrivals_are_announced_but_all_are_consumed(self):
        now = 1_700_000_000
        inbox = [
            {"id": 1, "created_at": now - 90_000, "title": "Deck complete", "body": "old"},
            {"id": 3, "created_at": now - 30, "title": "Deck complete", "body": "new"},
            {"id": 2, "created_at": now - 60, "title": "Deck complete", "body": "also new"},
        ]
        fresh, cursor = notify.fresh_messages(inbox, 0, now)
        self.assertEqual(["also new", "new"], [entry["body"] for entry in fresh])
        self.assertEqual(3, cursor)
        self.assertEqual(([], 3), notify.fresh_messages(inbox, cursor, now))

    def test_a_message_can_be_answered_until_it_has_been(self):
        self.assertTrue(notify.answerable({"sender": "cerro"}))
        self.assertFalse(notify.answerable({"sender": "cerro", "replied": True}))
        self.assertFalse(notify.answerable({"sender": ""}))


if __name__ == "__main__":
    unittest.main()
