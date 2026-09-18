import json
import urllib.parse
import urllib.request
from datetime import datetime

MAX_PENDING = 5000
PENDING_SQL = (
    "select id, cid, lastIvl, time, type from revlog "
    "where id > ? and ease > 0 and type < 4 order by id limit %d" % MAX_PENDING
)


def offset_west_min():
    offset = datetime.now().astimezone().utcoffset()
    return -int(offset.total_seconds() // 60) if offset else 0


def rows_to_reviews(rows):
    return [
        {"id": r[0], "cid": r[1], "last_ivl": r[2], "time_ms": r[3], "kind": r[4]}
        for r in rows
    ]


class Client:
    def __init__(self, url, user, token):
        self.base = url.strip().rstrip("/")
        self.user = urllib.parse.quote(user.strip(), safe="")
        self.token = token.strip()

    @property
    def configured(self):
        return bool(self.base and self.user and self.token)

    def _request(self, path, body=None):
        headers = {"Content-Type": "application/json"}
        if body is not None:
            headers["Authorization"] = "Bearer " + self.token
        request = urllib.request.Request(
            self.base + path,
            data=None if body is None else json.dumps(body).encode(),
            headers=headers,
        )
        with urllib.request.urlopen(request, timeout=20) as response:
            return json.load(response)


    def upload(self, reviews, rollover_hour, silent):
        return self._request(
            "/api/reviews/" + self.user,
            {
                "reviews": reviews,
                "clock": {
                    "offset_west_min": offset_west_min(),
                    "rollover_hour": rollover_hour,
                },
                "silent": silent,
            },
        )


def snapshot(profile):
    return {
        "xp": profile["xp_total"],
        "level": profile["level"],
        "into": profile["xp_into_level"],
        "need": profile["xp_for_next"],
        "streak": profile["streak"],
        "combo": profile["today"]["current_combo"],
        "quests": {q["title"] for q in profile["quests"] if q["done"]},
        "achievements": {a["title"] for a in profile["achievements"] if a["unlocked"]},
    }


def describe(before, after):
    gained = after["xp"] - before["xp"]
    if gained <= 0:
        return None
    lines = []
    if after["level"] > before["level"]:
        lines.append("Level %d!" % after["level"])
    lines += ["Achievement: " + t for t in sorted(after["achievements"] - before["achievements"])]
    lines += ["Quest complete: " + t for t in sorted(after["quests"] - before["quests"])]
    if after["streak"] > before["streak"]:
        lines.append("%d day streak" % after["streak"])
    status = "+%d XP" % gained
    if after["combo"] >= 5:
        status += "  ·  combo %d" % after["combo"]
    status += "  ·  Lv %d  %d/%d" % (after["level"], after["into"], after["need"])
    return "<br>".join(lines + [status]), bool(lines)
