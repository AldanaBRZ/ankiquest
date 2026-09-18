{
  description = "Gamification server for self-hosted Anki sync";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs = {
    self,
    nixpkgs,
  }: let
    lib = nixpkgs.lib;
    systems = ["x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin"];
    forAllSystems = f: lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
  in {
    formatter = forAllSystems (pkgs: pkgs.alejandra);

    packages = forAllSystems (pkgs: {
      default = pkgs.rustPlatform.buildRustPackage {
        pname = "ankiquest";
        version = "0.1.0";
        src = lib.cleanSource self;
        cargoLock.lockFile = ./Cargo.lock;
        meta.mainProgram = "ankiquest";
      };
    });

    devShells = forAllSystems (pkgs: {
      default = pkgs.mkShell {
        packages = with pkgs; [cargo clippy rustc rustfmt];
      };
    });

    nixosModules.default = {
      config,
      lib,
      pkgs,
      ...
    }: let
      cfg = config.services.ankiquest;
      syncMount = "/run/ankiquest-sync";
      withToken = lib.filterAttrs (_: u: u.tokenFile != null) cfg.users;
      user = lib.types.submodule {
        options = {
          display = lib.mkOption {
            type = lib.types.nullOr lib.types.str;
            default = null;
          };
          ntfyTopic = lib.mkOption {
            type = lib.types.nullOr lib.types.str;
            default = null;
            description = "ntfy topic for this player's notifications. Ends up in the world-readable store.";
          };
          tokenFile = lib.mkOption {
            type = lib.types.nullOr lib.types.str;
            default = null;
            description = "File holding the bearer token this player's AnkiDroid uploads reviews with.";
          };
        };
      };
    in {
      options.services.ankiquest = {
        enable = lib.mkEnableOption "ankiquest";
        package = lib.mkOption {
          type = lib.types.package;
          default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
        };
        port = lib.mkOption {
          type = lib.types.port;
          default = 8097;
        };
        domain = lib.mkOption {
          type = lib.types.nullOr lib.types.str;
          default = null;
          description = "Serve through nginx with ACME on this domain.";
        };
        syncBase = lib.mkOption {
          type = lib.types.nullOr lib.types.str;
          default = null;
          example = "/var/lib/private/anki-sync-server";
          description = "SYNC_BASE of a self-hosted Anki sync server to read reviews from, instead of or besides uploads.";
        };
        ntfy = lib.mkOption {
          type = lib.types.nullOr lib.types.str;
          default = null;
          example = "https://ntfy.sh";
        };
        remindHour = lib.mkOption {
          type = lib.types.ints.between 0 23;
          default = 20;
          description = "Local hour after which a streak-at-risk reminder is sent.";
        };
        users = lib.mkOption {
          type = lib.types.attrsOf user;
          default = {};
          description = "Players, keyed by name.";
        };
      };

      config = lib.mkIf cfg.enable {
        systemd.services.ankiquest = {
          description = "ankiquest";
          wantedBy = ["multi-user.target"];
          after = ["network.target" "anki-sync-server.service"];
          environment.ANKIQUEST_CONFIG = pkgs.writeText "ankiquest.json" (builtins.toJSON {
            addr = "127.0.0.1:${toString cfg.port}";
            sync_base =
              if cfg.syncBase == null
              then null
              else syncMount;
            state_dir = "/var/lib/ankiquest";
            ntfy = cfg.ntfy;
            remind_hour = cfg.remindHour;
            public_url =
              if cfg.domain == null
              then null
              else "https://${cfg.domain}";
            users =
              lib.mapAttrs (name: u: {
                display = u.display;
                ntfy_topic = u.ntfyTopic;
                token_file =
                  if u.tokenFile == null
                  then null
                  else "/run/credentials/ankiquest.service/token-${name}";
              })
              cfg.users;
          });
          serviceConfig = {
            ExecStart = lib.getExe cfg.package;
            DynamicUser = true;
            StateDirectory = "ankiquest";
            BindReadOnlyPaths = lib.optional (cfg.syncBase != null) "${cfg.syncBase}:${syncMount}";
            LoadCredential = lib.mapAttrsToList (name: u: "token-${name}:${toString u.tokenFile}") withToken;
            Restart = "always";
            RestartSec = 5;
          };
        };

        services.nginx.virtualHosts = lib.mkIf (cfg.domain != null) {
          ${cfg.domain} = {
            forceSSL = true;
            enableACME = true;
            locations."/".proxyPass = "http://127.0.0.1:${toString cfg.port}";
          };
        };
      };
    };
  };
}
