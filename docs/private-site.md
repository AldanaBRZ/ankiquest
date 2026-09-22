# Private AnkiQuest access

Private mode protects the whole community: leaderboard periods, profiles, records, Community history, and their APIs. The public sign-in page and its static assets contain no player data. Keep existing per-player tokens; members may sign in with a token or the server's shared website password.

## Server setup

1. Update the Android clients to a version that authenticates leaderboard/profile reads and signs the embedded website in with their saved token. Desktop add-on API requests already authenticate.
2. Create a private password file on the server if members should share a website password. Only the administrator and service should be able to read it.
3. Set `private_site: true` and, if using the shared password, `site_password_file` in the server JSON. In NixOS the options are `services.ankiquest.privateSite` and `services.ankiquest.sitePasswordFile`; the module passes the secret as a systemd credential, outside the Nix store.
4. Set the actual HTTPS `public_url` (or NixOS `domain`) and restart the service.
5. Check the site in a signed-out browser: it should show the sign-in page. Confirm that each app still loads standings and uploads reviews with its own token.

Do not distribute the shared password as an app upload credential. It gives shared website access but no personal permissions: it cannot upload reviews, change settings, or read a member's private messages. Configure apps with each player's own token. Leaving private mode disabled preserves existing public access. Enabling it with no usable credential fails at startup instead of making an inaccessible or accidentally public server.

## Browser sessions

Enter the shared password or a member token at the sign-in screen. The server issues a random session identifier; the password or token is not stored in the cookie. The cookie is HttpOnly, SameSite=Lax, scoped to `/`, and Secure when the configured public address uses HTTPS. Sessions expire after seven days and rotate when signing in again.

A token configured for exactly one player creates a member session for that player. Community, Activity, deck preferences, streak protection, incoming notification preferences, and profile-picture controls can reuse that session without repeated token entry. Personal endpoints accept either the owner's member cookie or the owner's bearer token; viewing another player's profile never grants permission to change their settings. These ownership rules also apply when the website is public.

The shared password always creates a view-only session with no member identity. This remains true if the same secret was also configured as a member token and is entered through the password form. A token shared by multiple players cannot choose a browser owner either; it can unlock shared pages, while explicit per-player bearer requests retain their existing behavior. Use distinct player tokens for personal browser sessions.

**Lock site** revokes the current session and clears its cookie. The website closes personal dialogs and clears their credentials and private presentation state on logout or account change. Protected responses use `Cache-Control: no-store`. Restarting the service invalidates all browser sessions and reloads password/token files, so restart when rotating credentials. Browser tokens are never placed in URLs, local storage, or session storage; manual fallback credentials stay only in page or dialog memory.

## Client API

| Request | Behavior |
| --- | --- |
| Existing data API, with `Authorization: Bearer <player-token>` | Works in private and public modes. Personal endpoints require the token belonging to the URL's player. An explicit invalid or different-player Authorization header never falls back to a member cookie. |
| Personal API, with the owner's member cookie | Authorizes that owner's reads and writes. Cookie-authenticated writes require `X-Ankiquest-CSRF: 1`; a supplied Origin must match the site. A shared-password cookie has no personal permissions. |
| Existing data API, without valid credentials in private mode | HTTP 401; no redirect to HTML and no player data. |
| `GET /auth/status` | Returns `private_site`, `authenticated`, and `member`. `member` identifies only the authenticated owner, or is `null`; passwords, tokens, and session identifiers are never returned. |
| `POST /auth/session`, player bearer token, empty body | HTTP 204 and `Set-Cookie`. A token with one owner establishes that member's session. Used by the Android embedded browser. |
| `POST /auth/session`, JSON `{ "password": "…" }`, `X-Ankiquest-CSRF: 1` | HTTP 204 and `Set-Cookie`. A unique member token creates its owner's session; the shared password or an ambiguous member token creates shared viewing access only. |
| `POST /auth/logout`, `X-Ankiquest-CSRF: 1` | HTTP 204; revokes the current cookie session and clears its cookie. A supplied Origin must match the site. |

For example, a valid member cookie or uniquely owned bearer token produces:

```json
{
  "private_site": true,
  "authenticated": true,
  "member": { "user": "alice", "display": "Alice" }
}
```

A shared-password session has `"authenticated": true` and `"member": null`. Without valid credentials, `authenticated` is `false` and `member` is `null`. Shared viewing authentication and owner identity are separate; an explicit invalid Authorization header suppresses cookie owner identity even if a valid viewing cookie is present. See [the member-session API guide](mobile-api.md) for Activity and challenge examples.

Session endpoints reject cross-origin browser submissions. Cookie-authenticated mutations require the CSRF header even in public mode; owner API mutations using a valid bearer token do not require it. `/auth/logout` always requires its explicit CSRF header. Failed password attempts are throttled per client IP (30 failures per minute), while valid bearer session bootstrap is not blocked by password failures. The NixOS nginx configuration overwrites `X-AnkiQuest-Client-IP` and enables `site_trust_proxy` automatically. For another reverse proxy, leave `site_trust_proxy` false unless the proxy connects over loopback and always overwrites that header with the real client address. The server ignores forwarded identity from non-loopback peers and when this setting is disabled. Protected responses and session responses are not cached. Credentials must travel over HTTPS on a deployed server. Never put permanent tokens or the shared password in links, query parameters, local storage, or session storage.

Older servers do not have `/auth/session`. Updated Android clients tolerate its HTTP 404 response and load their normal dashboard, preserving compatibility with those public servers.
