# Member sessions, Activity, and challenge events

These additions preserve bearer authentication and the existing notification array. XP, challenges' progress rules, review scheduling, streaks, and scoring are unchanged.

## Browser identity

`POST /auth/session` still returns `204` with the `ankiquest_session` cookie. A valid `Authorization: Bearer <member-token>` establishes that member's session. Browser JSON `{ "password": "..." }` accepts either the shared site password or a member token and requires `X-Ankiquest-CSRF: 1`. A shared-password session can read shared pages but has no personal permissions. If a secret is both the shared password and a member token, password login remains read-only. A token configured for multiple members cannot implicitly select a browser owner; existing explicit bearer requests retain their previous per-user behavior.

The cookie is HttpOnly, SameSite=Lax, scoped to `/`, and Secure when the configured public URL uses HTTPS. Sessions expire after seven days, rotate on sign-in, are revoked by logout, and disappear when the server restarts. Tokens never appear in status responses.

`GET /auth/status` returns:

```json
{
  "private_site": true,
  "authenticated": true,
  "member": { "user": "alice", "display": "Alice" }
}
```

`member` is `null` for a master session, an ambiguous token, or no valid member credential. Personal endpoints authorize only the URL's owner, through either their existing bearer token or their member cookie. An explicit invalid or different-member Authorization header never falls back to a cookie. Shared viewing and owner permission are separate, including when the server runs in public mode.

All cookie-authenticated mutations require `X-Ankiquest-CSRF: 1`. When Origin is present it must match the configured site's origin. The custom header also prevents a cross-site form submission when Origin is absent; the server does not enable cross-origin credentialed requests. Native bearer mutations do not require a CSRF header. `/auth/logout` retains its explicit header/origin check. Clients must clear personal snapshots and pending actions on logout/account change and scope caches to server plus member plus credential.

## Durable Activity

`GET /api/activity/{user}` accepts optional `days=90` (default) or `days=30`, `limit=100` (default, 1–200), and `before=<positive notification ID>`. It returns newest IDs first:

```json
{
  "items": [{
    "id": 42,
    "title": "Challenge invitation",
    "body": "alice invited you to Study together.",
    "day": 20718,
    "created_at": 1790000000,
    "sender": "alice",
    "replied": false,
    "kind": "challenge_invite",
    "read_at": null,
    "challenge_id": 8,
    "route": "/community#challenge-8",
    "action_required": true
  }],
  "unread_count": 1,
  "action_count": 1,
  "retention_days": 90,
  "window_days": 90,
  "next_before": null
}
```

Both timestamp fields are Unix **seconds**; challenge `start_at` and `end_at` retain their existing Unix **milliseconds** convention. Non-challenge rows have null `challenge_id` and `route`. Existing rows acquire null `read_at` and are unread. Activity preserves historical reminder text, including reminders that are no longer timely; it is not a source for replaying old system notifications.

Counts cover all items within the selected window, independently of pagination, push state, and any Android delivery cursor. `unread_count` counts rows with no read timestamp. `action_count` counts still-pending challenge invitations whose challenge is neither cancelled nor past its deadline. Reading an invitation does not accept it. A 30-day view counts its own 30-day window; use the default 90-day view for the full retained inbox. Follow `next_before` until null to load more.

`POST /api/activity/{user}/read` accepts `{ "ids": [42, 43] }` and returns `204`. At most 200 positive IDs per request; clients mark larger selections in batches. Duplicate IDs/retries are safe and preserve the first read timestamp. Unknown, expired, push-only, and other members' IDs have no effect. The caller must own `{user}`. Read state survives restart and changes neither push acknowledgement nor legacy delivery state.

The server physically retains notifications for 90 days; history already deleted by an older server cannot be reconstructed. The legacy `GET /api/notifications/{user}` remains an oldest-first array of up to 500 notifications from the last seven days, with its existing reminder delivery checks. It gains the four fields `read_at`, `challenge_id`, `route`, and `action_required`; old clients can ignore them. Push delivery eligibility also stays seven days. Push-only game events remain excluded from both personal inbox endpoints. New clients can fall back to the legacy endpoint after an Activity `404`, with no inferred unread badge.

## Challenge lifecycle

New event kinds are `challenge_invite`, `challenge_accepted`, and `challenge_complete`. The stable ID and relative route above are attached to each event. Only selected recipients receive invitations, only the creator receives acceptance events, and only accepted members receive completion events. No challenge creation broadcasts to unrelated members. Creation/membership changes, event deduplication, and notification insertion commit or roll back together. Completion is reconciled after uploads, in the regular server poll, and before personal challenge/inbox reads; completion notices are deduplicated across retries and restarts. Progress continues to use only reviews after joining and before the original deadline. A completion notice records that the goal was observed complete; later review undo or member departure can still alter live progress under the existing rules.

`POST /api/community/challenges/{user}` accepts optional `request_id`, an 8–128 character string of ASCII letters, numbers, hyphens, and underscores. New clients generate a fresh UUID for each deliberate creation and reuse it only when retrying that creation. A repeated key with the same normalized title, kind, cooperative flag, target, duration, and recipient set returns the existing challenge list without another challenge or event. Reusing it with different data returns `400`. Keys are scoped to the creator. Older clients may omit the key and retain their original create behavior.

Repeated successful accept, decline, leave, and creator cancel operations are idempotent. Accept retries never reset `joined_at`. Accepting or declining an invitation also marks that invitation read. Acting as a nonmember is forbidden; all challenge endpoints still require the URL owner's credential. Clients should reload challenge details before presenting available actions, since Activity can outlive an invitation, membership, or the challenge deadline.
