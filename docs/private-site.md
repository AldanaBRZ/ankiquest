# Private AnkiQuest access

Private mode protects the whole community: leaderboard periods, profiles, records, Community history, and their APIs. The public sign-in page and its static assets contain no player data. Keep existing per-player tokens; members may sign in with a token or the server's shared website password.

## Server setup

1. Update the Android clients to a version that authenticates leaderboard/profile reads and signs the embedded website in with their saved token. Desktop add-on API requests already authenticate.
2. Create a private password file on the server if members should share a website password. Only the administrator and service should be able to read it.
3. Set `private_site: true` and, if using the shared password, `site_password_file` in the server JSON. In NixOS the options are `services.ankiquest.privateSite` and `services.ankiquest.sitePasswordFile`; the module passes the secret as a systemd credential, outside the Nix store.
4. Set the actual HTTPS `public_url` (or NixOS `domain`) and restart the service.
5. Check the site in a signed-out browser: it should show the sign-in page. Confirm that each app still loads standings and uploads reviews with its own token.

Do not distribute the shared password as an app upload credential. It gives website access but cannot upload reviews or access another member's private settings or messages. Leaving private mode disabled preserves existing public access. Enabling it with no usable credential fails at startup instead of making an inaccessible or accidentally public server.

## Browser sessions

Enter the shared password or a member token at the sign-in screen. The server issues a random session identifier; the credential is not stored in the cookie. The cookie is HttpOnly, SameSite=Lax, scoped to the site, and Secure when the configured public address uses HTTPS. Sessions expire after seven days. **Lock site** revokes the current session. Restarting the service invalidates every browser session and reloads password/token files, so restart when rotating credentials.

Signing into the website does not connect a personal account in Community or unlock profile settings. Those actions retain their own player-token prompts. Tokens entered into those dialogs stay in memory for that connection only.

## Client API

| Request | Behavior |
| --- | --- |
| Existing data API, with `Authorization: Bearer <player-token>` | Works in private and public modes. Existing ownership checks remain on private member endpoints. |
| Existing data API, without valid credentials in private mode | HTTP 401; no redirect to HTML and no player data. |
| `GET /auth/status` | `{ "private_site": boolean, "authenticated": boolean }`; does not identify members or reveal credentials. |
| `POST /auth/session`, player bearer token, empty body | HTTP 204 and a `Set-Cookie` header for the website session. Used by the Android embedded browser. |
| `POST /auth/session`, JSON `{ "password": "…" }`, `X-Ankiquest-CSRF: 1` | Accepts a player token or shared website password and creates a read-access session. |
| `POST /auth/logout`, `X-Ankiquest-CSRF: 1` | Revokes the current cookie session and clears its cookie. |

Session endpoints reject cross-origin browser submissions. Failed password attempts are throttled per client IP (30 failures per minute). The NixOS nginx configuration overwrites `X-AnkiQuest-Client-IP` and enables `site_trust_proxy` automatically. For another reverse proxy, leave `site_trust_proxy` false unless the proxy connects over loopback and always overwrites that header with the real client address. The server ignores forwarded identity from non-loopback peers and when this setting is disabled. Protected responses and session responses are not cached. Credentials must travel over HTTPS on a deployed server. Never put permanent tokens or the shared password in links, query parameters, or local storage.

Older servers do not have `/auth/session`. Updated Android clients tolerate its HTTP 404 response and load their normal dashboard, preserving compatibility with those public servers.
