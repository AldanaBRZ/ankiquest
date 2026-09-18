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
          type = lib.types.str;
          default = "/var/lib/private/anki-sync-server";
          description = "SYNC_BASE of the Anki sync server, holding one folder per sync user.";
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
          description = "Optional per-player settings, keyed by sync username. Every sync user is a player either way.";
        };
      };

      config = lib.mkIf cfg.enable {
        systemd.services.ankiquest = {
          description = "ankiquest";
          wantedBy = ["multi-user.target"];
          after = ["network.target" "anki-sync-server.service"];
          environment.ANKIQUEST_CONFIG = pkgs.writeText "ankiquest.json" (builtins.toJSON {
            addr = "127.0.0.1:${toString cfg.port}";
            sync_base = syncMount;
            state_dir = "/var/lib/ankiquest";
            ntfy = cfg.ntfy;
            remind_hour = cfg.remindHour;
            public_url =
              if cfg.domain == null
              then null
              else "https://${cfg.domain}";
            users =
              lib.mapAttrs (_: u: {
                display = u.display;
                ntfy_topic = u.ntfyTopic;
              })
              cfg.users;
          });
          serviceConfig = {
            ExecStart = lib.getExe cfg.package;
            DynamicUser = true;
            StateDirectory = "ankiquest";
            BindReadOnlyPaths = ["${cfg.syncBase}:${syncMount}"];
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
