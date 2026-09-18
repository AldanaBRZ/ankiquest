# ankiquest

XP, levels, streaks, daily quests, achievements and a leaderboard for Anki, computed on the server from the review log of a self-hosted sync server. Works with AnkiDroid, desktop and iOS because nothing runs on the client.

XP never depends on which answer button was pressed, so there is no incentive to grade dishonestly.

## Run

```sh
cargo run -- ankiquest.json
```

```json
{
  "addr": "127.0.0.1:8097",
  "sync_base": "/path/to/SYNC_BASE",
  "state_dir": "state",
  "ntfy": "https://ntfy.sh",
  "remind_hour": 20,
  "public_url": "https://anki.example.com",
  "users": { "hill": { "display": "hill", "ntfy_topic": "some-secret-topic" } }
}
```

Only `sync_base` is required. Every folder in it with a `collection.anki2` becomes a player; collections are copied before reading and never written. Open `/#<user>` for a profile, `/` for the leaderboard.

## NixOS

```nix
inputs.ankiquest.url = "github:float3/ankiquest";

imports = [inputs.ankiquest.nixosModules.default];

services.anki-sync-server = {
  enable = true;
  users = [
    {
      username = "hill";
      passwordFile = "/etc/nixos/secrets/anki-sync-hill";
    }
  ];
};
services.nginx.virtualHosts."ankisync.example.com" = {
  forceSSL = true;
  enableACME = true;
  locations."/" = {
    proxyPass = "http://127.0.0.1:${toString config.services.anki-sync-server.port}";
    extraConfig = "client_max_body_size 0;";
  };
};
services.ankiquest = {
  enable = true;
  domain = "anki.example.com";
  ntfy = "https://ntfy.sh";
  users.hill.ntfyTopic = "some-secret-topic";
};
```

In AnkiDroid: Settings → Sync → Custom sync server → `https://ankisync.example.com/`.
