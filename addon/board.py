"""The leaderboard as it appears under the deck list, and the periods it can show."""

PERIODS = [
    ("hour", "Hour"),
    ("day", "Today"),
    ("week", "Week"),
    ("month", "Month"),
    ("year", "Year"),
    ("all", "All time"),
]
DEFAULT_PERIOD = "week"


def label(period):
    return dict(PERIODS).get(period, dict(PERIODS)[DEFAULT_PERIOD])


def xp_of(row, period):
    periods = row.get("periods") or {}
    if period in periods:
        return periods[period]
    return row.get("week_xp", 0)


def ranked(board, period):
    """The same order the app and the website use: most experience first."""
    return sorted(
        board,
        key=lambda row: (-xp_of(row, period), (row.get("display") or row["user"]).lower()),
    )


def escape(text):
    return (
        str(text)
        .replace("&", "&amp;")
        .replace("<", "&lt;")
        .replace(">", "&gt;")
        .replace('"', "&quot;")
    )


def tabs(period):
    links = []
    for name, text in PERIODS:
        style = "font-weight:bold;text-decoration:underline" if name == period else "opacity:.6"
        links.append(
            "<a href=# style='%s;margin-left:.6em' onclick=\"pycmd('ankiquest:period:%s');return false\">%s</a>"
            % (style, name, escape(text))
        )
    return "".join(links)


def rows(board, period, me):
    out = []
    for index, row in enumerate(ranked(board, period)):
        mine = row["user"] == me
        rank = "\U0001f451" if index == 0 else str(index + 1)
        name = escape(row.get("display") or row["user"])
        streak = "\U0001f525 %d" % row["streak"] if row.get("streak") else ""
        out.append(
            "<tr style='%s'>"
            "<td style='width:2.5em;text-align:center'>%s</td>"
            "<td>%s</td>"
            "<td style='text-align:right;opacity:.7;white-space:nowrap'>Lv %d</td>"
            "<td style='text-align:right;opacity:.7;white-space:nowrap'>%s</td>"
            "<td style='text-align:right;white-space:nowrap'>%s XP</td>"
            "</tr>"
            % (
                "font-weight:bold" if mine else "",
                rank,
                name,
                row.get("level", 1),
                streak,
                "{:,}".format(xp_of(row, period)),
            )
        )
    return "".join(out)


def html(board, period, me, profile=None, unread=0):
    """A block for `deck_browser_will_render_content`, so it lands under the stats."""
    period = period if period in dict(PERIODS) else DEFAULT_PERIOD
    if not board:
        body = "<tr><td style='opacity:.6;padding:.6em 0'>No one has any experience yet.</td></tr>"
    else:
        body = rows(board, period, me)
    footer = ""
    if profile:
        parts = [
            "Lv %d" % profile.get("level", 1),
            "%s/%s XP" % (
                "{:,}".format(profile.get("xp_into_level", 0)),
                "{:,}".format(profile.get("xp_for_next", 0)),
            ),
        ]
        if profile.get("streak"):
            parts.append("\U0001f525 %d day streak" % profile["streak"])
        if profile.get("at_risk"):
            parts.append("streak at risk today")
        footer = "<div style='opacity:.7;margin-top:.5em'>%s</div>" % escape("  ·  ".join(parts))
    inbox = ""
    if unread:
        inbox = (
            "<a href=# style='margin-left:.6em' onclick=\"pycmd('ankiquest:inbox');return false\">"
            "\U0001f4ec %d new</a>" % unread
        )
    return (
        "<div id=ankiquest style='max-width:600px;margin:2em auto 0;text-align:left;font-size:13px'>"
        "<div style='display:flex;justify-content:space-between;align-items:baseline;"
        "border-bottom:1px solid;padding-bottom:.3em;margin-bottom:.3em'>"
        "<div><b>Leaderboard</b> <span style='opacity:.7'>%s</span>%s</div>"
        "<div>%s</div></div>"
        "<table style='width:100%%;border-collapse:collapse'>%s</table>%s</div>"
        % (escape(label(period)), inbox, tabs(period), body, footer)
    )
