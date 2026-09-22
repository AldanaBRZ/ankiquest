# Community, reminders, and challenges

Open **Community** from the leaderboard, or visit `/community`. Public history can be explored without signing in. Choose a player, year, and, in the daily calendar, a month.

For **Reminders** or **Challenges**, the updated AnkiDroid dashboard connects with the account already saved in the app. **Settings → ankiquest → Community reminders** opens the reminder controls directly. In a standalone browser, select **Connect account** and use the player's existing AnkiQuest bearer token. Tokens stay in page memory only, never URLs or browser storage. Disconnecting stops private requests until you explicitly connect again; AnkiDroid can reuse its saved account for that reconnect. Each account can manage only its own preferences and invitations.

These features become available after the updated server is installed and restarted.

## The 18 features

| # | Feature | Where and how it works |
| --- | --- | --- |
| 1 | Gentle daily reminder | **Reminders**: a nudge at or after the chosen hour if no studying has been recorded for the current Anki day. |
| 2 | Urgent streak warning | Warns when an existing streak is at risk and no freeze can protect it, including when protection is switched off. |
| 3 | Freeze-used notice | Reports a newly consumed freeze and the remaining balance. |
| 4 | Freeze refill reminder | Prompts unfinished daily quests when protection is enabled, storage has room, and today's quest freeze has not been earned. |
| 5 | Upcoming streak milestone | Encouragement one study day before milestones such as 7, 30, 50, 100, and 365 days. |
| 6 | Weekly competition closing | An optional warning before the shared week ends, for players with weekly XP. |
| 7 | Weekly recap | Study days, XP, daily wins, weekly rank, and XP change from the previous week, after the result is finalized. |
| 8 | Winners calendar | **Calendar**: daily results, weekly champions, and monthly seasons. Open a period for standings, XP, reviews, study time, new cards, and winning margin. |
| 9 | Trophy cabinet | **Trophies**: first daily, weekly, and monthly victories; seven and thirty daily wins; consistency and comeback trophies. |
| 10 | Most improved | **Overview / Trophies**: the greatest positive increase in review count between consecutive complete, finalized weeks. Ties share the award. |
| 11 | Consistency awards | Studying on all seven shared competition dates in a complete week. Freezes do not count as study days. |
| 12 | Monthly seasons | A new calendar-month competition with archived standings; lifetime trophies remain. |
| 13 | Friendly challenges | **Challenges**: invite friends to personal study-day or review targets. Each invitee chooses whether to accept. |
| 14 | Group goals | Choose **One shared group goal** to combine participants' reviews or study days toward a common target. |
| 15 | Head-to-head history | **Records**: compare any two players' daily and weekly scores, independent of the overall winner. |
| 16 | Comeback recognition | Celebrate a return after at least seven missed competition dates. |
| 17 | Record timeline | **Records**: personal and server bests for daily XP, daily review count, and streak length. Filter by player, year, and record scope. |
| 18 | Year in review | **Year review**: study days, reviews, XP, best streak, monthly progress, strongest month, wins, and trophies earned that year. |

## Reminder timing and delivery

All seven reminder types are **off by default**. Default timing is 20:00, quiet hours are 22:00–09:00, and the daily cap is two reminders. Hours accept 0–23 and the cap accepts 1–5. Identical quiet-start and quiet-end hours disable quiet time.

Timing uses the player's synced Anki clock and day cutoff. Urgent streak and weekly-closing warnings normally use the last two hours before the relevant deadline. If quiet time covers that deadline, they can arrive in the last waking hour instead: a 04:00 cutoff with 22:00–09:00 quiet time produces a warning around 21:00. Quiet hours and the cap still apply to urgent reminders; a slot is reserved for an enabled urgent warning when an unprotected streak is at risk.

Daily, milestone, and urgent reminders are rechecked against current studying before delivery. Duplicate reminders are suppressed, including across restarts. Changing preferences preserves pending reminders that remain enabled and rechecks their timing; disabling a type cancels its pending notices. A milestone reminder replaces a generic daily nudge for that day. Enabling freeze-used notices or recaps does not announce historical events retroactively.

Reminders use the existing authenticated notification inbox and durable push queue. Push delivery requires the server's `ntfy` destination and the player's `ntfy_topic`; queued attempts are rechecked and retried. These preferences control server reminders. Device-only AnkiDroid and desktop add-on alarms remain independently controlled in each client's settings. The old global `remind_hour` / Nix `remindHour` setting is superseded by personal preferences.

## What the historical record means

The archive uses one shared competition timezone and cutoff, initially taken from `week_timezone` and `week_rollover_hour` (Nix: `weekTimezone`, `weekRolloverHour`). Each full Anki day's XP belongs to the shared date containing that Anki day's start, matching the existing weekly leaderboard. Weeks start on Monday and seasons start on the first of the month.

Set `competition_start_date` to the known server start date as `YYYY-MM-DD` before initializing the archive; the Nix option is `competitionStartDate`. Otherwise, the archive uses the earliest retained review history, which can predate this server. It does not invent an exact server start date or claim missing history is complete. The archive records its original timezone and cutoff so later configuration changes cannot silently relabel finalized periods.

When `sync_base` is configured, archival updates wait for a complete collection import. Unreadable collections or files changing during import defer finalization until a later successful attempt. The community endpoint temporarily returns HTTP 503 during an incomplete import, while existing notification processing continues.

| Display status | Meaning |
| --- | --- |
| Live | The competition is still open; displayed winners are current leaders. |
| Pending / Awaiting finalization | The cutoff passed, but a 24-hour allowance remains for late syncs. |
| Final | The allowance ended. The stored result is immutable. Later syncs can still affect the live leaderboard. |
| Reconstructed / Rebuilt | A historical period was reconstructed from review history available when archived. |
| Partial period | The period begins before available competition history, so its opening portion is missing. |

Days without studying have no winner. Every tied champion receives a win; tied-win totals include daily, weekly, and monthly competitions. Lifetime counters, awards, and records use finalized results. Yearly activity totals also include current and pending periods. Head-to-head comparisons use periods in which both players were present and exclude periods where neither studied. Weekly head-to-head results require a full shared week for that pair, with both represented in every finalized daily snapshot; another player's later arrival does not shorten their pairwise history.

## Fair comparisons and lifetime wins

The main leaderboard and the Community trophy table default to **Shared history**. Daily, weekly, and monthly win totals use the same comparison window for everyone: the latest first recorded study date among current players with archived study history. For example, when one player's history starts in 2019 and another's starts in 2022, their shared totals begin in 2022. The earlier solo years remain available under **Lifetime**.

Shared history needs at least two players with study history. Accounts without recorded studying are listed as waiting and do not move the comparison date. The window updates when another player gains history; it is a comparison of the current group, not a permanent lifetime ranking. Each player's first recorded date is visible, and missing history is never treated as proof of when they joined the server.

Only finalized, complete periods starting on or after the shared date count. Weeks and months spanning that date are excluded, so a newcomer never competes against another player's earlier days in the same week or month. Every player must be represented in each archived daily snapshot throughout the period. A represented player who skipped studying still participates; a missing snapshot does not create a loss. Days, weeks, or months without a winner do not count as competitions. Tied champions each receive a win.

**Lifetime** shows all finalized wins from available history, including earlier solo wins and partial opening weeks or months. These totals preserve the archive rather than offering an equal-opportunity ranking. Historical winners are never recalculated for a smaller group; if a former player won, that result stays theirs. The calendar and trophies retain the original results in both views.

## Creating and leaving challenges

Choose a name, measure, target, duration, and 1–30 friends. Names allow 1–80 characters; duration is 1–31 days; targets are 1–100,000. A study-day target must fit the duration and participant count. Creators can have at most ten uncancelled challenges whose deadlines have not passed.

Challenges start immediately. The creator is accepted automatically; invited players' activity counts only after acceptance. Review progress excludes future timestamps and activity at or after the deadline. Study days use each participant's Anki clock, and a shared goal counts each participant's study day separately.

Only accepted or invited members can retrieve the challenge list. Invitees can accept or decline; accepted invitees can leave; the creator can cancel. Declining or leaving removes the challenge from that player's list and removes their contribution from the shared total. A reached goal can still be cancelled before its deadline, freeing an open-challenge slot. Cancelled and expired challenges preserve their details for remaining members.

All open memberships remain accessible; the history limit applies only to the 100 most recent cancelled or expired challenges.

## API reference

Private routes require `Authorization: Bearer <token>` for the `{user}` in the path. JSON writes reject unknown fields.

| Route | Payload / result |
| --- | --- |
| `GET /api/community?year=2026&month=9` | Public `{meta, players, calendar, weeks, seasons, awards, records, head_to_head, winner_history}`. Player counters cover all time; `year_review`, awards, and records use the selected year; `calendar` uses the selected month. |
| `GET /api/winners` | Public `{meta, shared, lifetime, waiting_players}`. Both scopes include player identities, first recorded study dates, and `day_wins`, `week_wins`, `month_wins`; `shared` also includes the comparison start date, player count, and eligible competition counts. Uses the same archive and import safeguards as `/api/community`. |
| `GET /api/community/reminders/{user}` | Current reminder settings. |
| `POST /api/community/reminders/{user}` | Full replacement: booleans `gentle_daily`, `urgent_streak`, `freeze_used`, `freeze_refill`, `milestone`, `weekly_closing`, `weekly_recap`; numeric `reminder_hour`, `quiet_start`, `quiet_end`, `daily_limit`. Returns saved settings. |
| `GET /api/community/challenges/{user}` | `{challenges, recipients}`; recipients are eligible configured accounts. |
| `POST /api/community/challenges/{user}` | `{title, kind, cooperative, target, duration_days, recipients}`. `kind` is `study_days` or `reviews`; `recipients` contains player IDs. Returns the updated list. |
| `POST /api/community/challenges/{user}/{id}` | `{action: "accept" \| "decline" \| "leave" \| "cancel"}`. Returns the updated list. |

Competition dates are ISO dates; period `end` is exclusive. Daily and weekly keys are `YYYY-MM-DD`, and monthly keys are `YYYY-MM`. Challenge `start_at` and `end_at` are Unix **milliseconds**. An empty installation returns no invented winners and can have `meta.start_date: null`.
