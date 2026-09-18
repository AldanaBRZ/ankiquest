# ankiquest

XP, levels, streaks, daily quests, achievements and a leaderboard for Anki. The server only ever sees review log rows (card id, timestamp, previous interval, time taken, review type), never card content.

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
  "users": {
    "hill": { "display": "hill", "ntfy_topic": "some-secret-topic", "token_file": "hill.token" }
  }
}
```

Open `/#<user>` for a profile, `/` for the leaderboard.

## Getting reviews in

The [AnkiDroid fork](https://github.com/float3/Anki-Android/tree/ankiquest) uploads new review rows after each answer and shows XP feedback while reviewing. Set the server URL, player and token under Settings → Sync → Custom sync server. Sync itself can stay on AnkiWeb.

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

## NixOS

```nix
inputs.ankiquest.url = "github:float3/ankiquest";

imports = [inputs.ankiquest.nixosModules.default];

services.ankiquest = {
  enable = true;
  domain = "anki.example.com";
  ntfy = "https://ntfy.sh";
  users.hill = {
    tokenFile = "/etc/nixos/secrets/ankiquest-hill";
    ntfyTopic = "some-secret-topic";
  };
};
```
