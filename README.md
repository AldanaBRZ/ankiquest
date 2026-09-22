# ankiquest

XP, levels, streaks, daily quests, achievements and a leaderboard for Anki. Clients send review log rows (card id, timestamp, previous interval, time taken, review type), never card content. Deck names and daily counts are only sent by players who use deck completion notifications.

XP never depends on which answer button was pressed, so there is no incentive to grade dishonestly.

## Run

```sh
cargo run -- ankiquest.json
```

```json
{
  "addr": "127.0.0.1:8097",
  "state_dir": "state",
  "ntfy": "https://ntfy.sh",
  "remind_hour": 20,
  "public_url": "https://anki.example.com",
  "week_timezone": "Europe/Berlin",
  "week_rollover_hour": 4,
  "users": {
    "hill": { "display": "hill", "ntfy_topic": "some-secret-topic", "token_file": "hill.token" }
  }
}
```

Open `/#<user>` for a profile, `/` for the leaderboard. `/hour`, `/day`, `/week`, `/month`, `/year` and `/all` show the same board for another period, as does `GET /api/leaderboard?period=<name>`; each standing carries `xp` for the requested period and `periods` with all of them. The hour is the last 60 minutes and counts review XP only, the day is each player's own Anki day, and month and year follow the calendar in `week_timezone`. The leaderboard week runs Monday to Sunday in `week_timezone` and turns over at `week_rollover_hour` for everyone at once; each Anki day counts towards the week it started in. Streaks, quests and "today" still follow each player's own Anki day.

`/records` and `GET /api/records` name whoever has had the best hour, day, week, month and year here, with the XP and the review count of each and the two who came closest, along with the longest streak and the most days studied; a profile shows the same as personal bests. The record hour is any 60 minutes, not a clock hour.

## Getting reviews in

Both clients upload new review rows after each answer and show XP feedback while reviewing. Sync itself can stay on AnkiWeb.

- AnkiDroid: install the [fork](https://github.com/float3/Anki-Android/tree/ankiquest) and fill in Settings → ankiquest.
- Desktop: zip the contents of `addon/` into `ankiquest.ankiaddon`, open it with Anki, then fill in **Tools → ankiquest settings…**. The leaderboard appears under the deck list, with the period links it shares with the website.

`POST /api/reviews/<user>` with `Authorization: Bearer <token>` and

```json
{
  "reviews": [{ "id": 0, "cid": 0, "last_ivl": 0, "time_ms": 0, "kind": 0 }],
  "clock": { "offset_west_min": -120, "rollover_hour": 4 },
  "silent": false
}
```

stores the rows and returns the profile. `POST /api/preview/<user>` takes the same `reviews` without storing anything.

Alternatively set `sync_base` to the `SYNC_BASE` of a self-hosted Anki sync server: every folder in it with a `collection.anki2` becomes a player, and collections are copied before reading and never written.

## Streak freezes

Streak freezes are off by default and start at zero. Open your dashboard profile, choose **Manage streak freezes**, and enter your AnkiQuest token to opt in. Complete all three daily quests while enabled to earn one freeze per Anki day, up to three stored. This replaces the automatic freeze awarded every seven study days. On an existing server, previously protected days and their streak/XP history are preserved; unused automatic freezes are cleared on the upgrade's Anki day. Enabling freezes does not award any for earlier quest completions, including earlier today; completing extra reviews or toggling the setting cannot claim that day's reward again. Completing the quests with a full inventory does not bank a fourth freeze for later.

When an Anki day ends with no reviews, one available freeze automatically protects an existing streak. A protected day preserves the streak count without adding a study day. Consecutive missed days each need one freeze; once there is none available, the next missed day resets the streak. Turning freezes off pauses earning and spending, keeps stored freezes, and leaves previously protected days intact. Enabling them again cannot repair days missed while they were off. Day boundaries follow the player's Anki timezone and rollover, not midnight on the server.

`GET /api/streak-freezes/<user>` and `POST` with `{"enabled":true}` or `{"enabled":false}` require that player's bearer token and return `{"enabled":true,"freezes":0,"capacity":3}`. The server calculates the balance from reviews and the saved preference timeline; clients cannot set it. The public profile includes `freezes_enabled`, `stored_freezes`, and `freeze_earned_today`. Its existing `freezes` field is the available balance (zero while disabled), so older clients do not mistake paused stock for active protection. Review previews can show a projected reward but do not save it. As with XP and quests, importing or deleting review history recalculates the result.

## Deck completion notifications

Notifications are off by default for every deck. To set them up, tick the decks you want to share and the people to notify: in AnkiDroid under **Settings → ankiquest → Deck completion notifications**, on desktop under **Tools → ankiquest deck notifications…**, or on the dashboard through **Manage deck notifications** with your upload token. Ticking a deck ticks its subdecks.

After you review at least one card in a deck and finish its scheduled work for the day, selected people receive a message such as “cerro has finished their Spanish studies for today.” A parent deck includes its subdecks. Daily limits are respected, and learning cards due later that day still count as unfinished work. Each deck is announced at most once per Anki day, using your Anki day rollover, even across retries or a server restart. Turning sharing on after finishing a deck does not send a retrospective announcement.

Recipients receive announcements through the updated AnkiDroid client's background notification checks (roughly every 15 minutes, subject to Android's background limits), on desktop through the add-on's own check every five minutes, and through their configured ntfy topic when available (the server checks every 20 seconds). Each announcement can be answered once, with a cheer or your own words: from the Android notification itself, or from **Tools → ankiquest inbox…** on desktop. `POST /api/reply/<user>` with `{"notification": 1, "message": "Good job!"}` delivers the answer, which can be answered in turn. The upload response lists what it just announced, so the client that finished a deck can say who was told. Deck names and recipient preferences are private to the authenticated player; only selected recipients receive the completion message. The dashboard keeps the token only while the settings window is open.

Players can opt into nudges through **Manage deck notifications** on the dashboard. When a place on the weekly board, their best day ever or the next level is within 150 XP, they hear about it once a day each, in reviews as well as XP. Nudges only arrive between 9:00 and 22:00 of a player's own day, and only after they have already reviewed something, so they never tell anyone to start studying.

Messages can also be written by hand on the server: `sudo ankiquest-message --from cerro aldanita "you are doing great, keep going"`, or `ankiquest <config> message <player> <text>` without the NixOS module. They arrive like any other notification, and with `--from` the recipient can answer them.

An ntfy push is marked delivered only after a successful HTTP response. Pushes request high priority for vibration and pop-up alerts, subject to the phone's notification settings. Failed pushes retry after 20 seconds, backing off to at most 15 minutes; missing ntfy configuration leaves inbox messages pending. The queue and inbox retain messages for seven days. Retries take turns between recipients and limit network work to 20 seconds per server check. Unlocks and streak warnings retain their existing ntfy-only delivery; they do not add duplicate inbox alerts. A queued streak warning is cancelled when its Anki day ends or the player studies. A server restart or a lost HTTP response can occasionally cause a duplicate push, but retries keep the same inbox notification.

The server and the client used to study must both be updated. Clients only report progress for decks you share, so players who never enable a deck send no deck data. Sending the deck list again replaces the stored one, which removes deleted decks.

Clients may include a `decks` array in the review upload. Each entry has `id` (a string), `name`, `remaining`, `reviewed_today`, and `day` (the local Anki day number, days since the Unix epoch after applying timezone and rollover). Omit this field when a reliable snapshot is unavailable. Initial silent uploads populate the deck list without announcing completions. Set `"catalog": true` when `decks` is the full deck list; decks missing from it are removed.

`GET /api/decks/<user>` with the user's bearer token returns private deck preferences and available recipients. `POST` to the same endpoint accepts `{"decks":[{"id":"123","enabled":true,"recipients":["hill"]}]}`. `GET /api/notifications/<user>` with the recipient's bearer token returns their recent completion announcements, with `id`, `title`, `body`, `day`, and `created_at` (Unix seconds). These endpoints never expose another player's deck settings or notification inbox without that player's token.

## NixOS

```nix
inputs.ankiquest.url = "github:float3/ankiquest";

imports = [inputs.ankiquest.nixosModules.default];

services.ankiquest = {
  enable = true;
  domain = "anki.example.com";
  weekTimezone = "Europe/Berlin";
  ntfy = "https://ntfy.sh";
  users.hill = {
    tokenFile = "/etc/nixos/secrets/ankiquest-hill";
    ntfyTopic = "some-secret-topic";
  };
};
```
