/* Shared navigation, presentation components, and the browser access gate. */
(() => {
  "use strict";
  const embedded = new URLSearchParams(location.search).get("embed") === "1";
  document.documentElement.classList.toggle("embedded", embedded);
  const escape = value => String(value ?? "").replace(/[&<>"']/g, char => ({"&":"&amp;","<":"&lt;",">":"&gt;",'"':"&quot;","'":"&#39;"}[char]));
  const href = (path, hash = "") => `${path}${embedded ? (path.includes("?") ? "&" : "?") + "embed=1" : ""}${hash}`;
  const avatar = (user, display = user) => {
    if (window.AnkiQuestAvatars) return AnkiQuestAvatars.markup(user, display);
    let hash = 0;
    for (const char of String(user)) hash = (hash * 31 + char.charCodeAt(0)) | 0;
    const initials = String(display || user).trim().split(/\s+/).map(part => Array.from(part)[0] || "").slice(0, 2).join("").toUpperCase();
    return `<span class="avatar" style="border-color:hsl(${Math.abs(hash) % 360} 55% 55%)" aria-hidden="true">${escape(initials)}</span>`;
  };
  const kpi = (label, value, detail, tone = "") => `<div class="kpi ${escape(tone)}"><span class="kpi-label">${escape(label)}</span><strong>${escape(value)}</strong><span>${escape(detail)}</span></div>`;
  let statusPromise, lastIdentity, statusEpoch = 0;
  const memberUser = access => typeof access?.member?.user === "string" ? access.member.user : null;
  async function status(fresh = false) {
    if (fresh) { statusPromise = undefined; statusEpoch++; }
    if (!statusPromise) statusPromise = fetch("/auth/status", {cache: "no-store", credentials: "same-origin"})
      .then(response => response.ok ? response.json() : null).catch(() => null);
    const epoch = statusEpoch, pending = statusPromise;
    const access = await pending;
    if (epoch !== statusEpoch) return status();
    if (access) {
      const user = memberUser(access);
      if (lastIdentity !== user) {
        lastIdentity = user;
        dispatchEvent(new CustomEvent("ankiquest:identity", {detail: {user}}));
      }
      if (user) setProfile(user);
    } else statusPromise = undefined;
    return access;
  }
  async function member(user) {
    const identity = memberUser(await status());
    return identity && (!user || identity === user) ? {user: identity} : null;
  }
  const ownerHeaders = (session, body) => ({
    ...(session?.token ? {Authorization: "Bearer " + session.token} : {}),
    ...(body === undefined ? {} : {"Content-Type": "application/json", "X-Ankiquest-CSRF": "1"}),
  });
  // A token is kept only for older servers which do not issue an owner session.
  async function connectMember(user, token) {
    const owner = await fetch("/api/community/reminders/" + encodeURIComponent(user), {cache: "no-store", headers: {Authorization: "Bearer " + token}});
    if (!owner.ok) throw new Error("This token was not accepted for the selected player. Check the player and token.");
    const response = await fetch("/auth/session", {method: "POST", credentials: "same-origin", headers: {Authorization: "Bearer " + token, "X-Ankiquest-CSRF": "1"}});
    if (response.status === 404) return {user, token};
    if (!response.ok) throw new Error("Could not connect your account. Check the token and try again.");
    const access = await status(true), identity = memberUser(access);
    if (identity && identity !== user) {
      try { await disconnectMember(); }
      finally { dispatchEvent(new Event("ankiquest:locked")); }
      throw new Error("This token belongs to a different player. Reconnect with your own player and token.");
    }
    return identity ? {user: identity} : {user, token};
  }
  async function disconnectMember() {
    const response = await fetch("/auth/logout", {method: "POST", headers: {"X-Ankiquest-CSRF": "1"}});
    if (!response.ok && response.status !== 404) throw new Error("Could not disconnect. Check your connection and try again.");
    statusPromise = undefined; lastIdentity = undefined; statusEpoch++;
    try { localStorage.removeItem("ankiquestPlayer"); } catch (_) {}
    dispatchEvent(new Event("ankiquest:locked"));
    return refreshAccess(true);
  }
  const loginURL = () => "/login?next=" + encodeURIComponent(location.pathname + location.search + location.hash);
  async function refreshAccess(fresh = false) {
    const access = await status(fresh);
    document.querySelectorAll(".site-lock").forEach(button => button.hidden = !access?.private_site && !memberUser(access));
    if (memberUser(access)) setProfile(memberUser(access));
    if (access?.private_site && !access.authenticated) {
      dispatchEvent(new Event("ankiquest:locked"));
      location.replace(loginURL());
    }
    return access;
  }
  async function checkAccess(response, options = {}) {
    if (response.status === 401 && !new Headers(options.headers).has("Authorization")) {
      const access = await status(true);
      if (access?.private_site && !access.authenticated) {
        dispatchEvent(new Event("ankiquest:locked"));
        location.replace(loginURL());
      }
    }
  }
  async function readJSON(url, options = {}) {
    const response = await fetch(url, {cache: "no-store", ...options});
    await checkAccess(response, options);
    if (!response.ok) throw new Error(`The server could not load this page (${response.status}).`);
    return response.json();
  }
  function setProfile(user, current = false) {
    const target = lastIdentity || user;
    document.querySelectorAll("[data-profile-link]").forEach(link => {
      link.hidden = !target;
      link.textContent = lastIdentity ? "My profile" : "Profile";
      link.href = href("/week", "#" + encodeURIComponent(target));
      if (current && target === user) link.setAttribute("aria-current", "page");
      else link.removeAttribute("aria-current");
    });
  }
  async function mountHeader() {
    const header = document.querySelector("[data-site-header]");
    if (!header) return;
    const actions = header.querySelector("[data-site-actions]");
    const active = header.dataset.siteSection || "leaderboard";
    const links = [["leaderboard", "/week", "Leaderboard"], ["records", "/records", "Records"], ["community", "/community", "Community"]];
    header.innerHTML = `<a class="brand" href="${href("/")}" aria-label="AnkiQuest home"><img src="/icon.svg" alt="">ankiquest</a><nav class="site-nav" aria-label="Main navigation">${links.map(([key, path, label]) => `<a href="${href(path)}"${key === active ? ' aria-current="page"' : ""}>${label}</a>`).join("")}</nav><div class="top-actions"><a data-profile-link hidden>Profile</a><button type="button" class="site-lock" hidden>Lock site</button></div><p class="site-status error" role="status" hidden></p>`;
    if (embedded) header.insertAdjacentHTML("afterend", `<nav class="tabs embedded-nav" aria-label="Main navigation">${links.map(([key, path, label]) => `<a href="${href(path)}"${key === active ? ' aria-current="page"' : ""}>${label}</a>`).join("")}</nav>`);
    const controls = header.querySelector(".top-actions");
    if (actions) while (actions.firstChild) controls.insertBefore(actions.firstChild, controls.querySelector(".site-lock"));
    try { setProfile(localStorage.getItem("ankiquestPlayer") || ""); } catch (_) {}
    const lock = header.querySelector(".site-lock"), message = header.querySelector(".site-status");
    lock.addEventListener("click", async () => {
      lock.disabled = true;
      message.hidden = true;
      try {
        const access = await disconnectMember();
        if (!access?.private_site) location.replace(loginURL());
      } catch (error) {
        message.textContent = error.message;
        message.hidden = false;
        lock.disabled = false;
      }
    });
    await refreshAccess();
  }
  window.AnkiQuestSite = {embedded, escape, href, avatar, kpi, setProfile, checkAccess, readJSON, status, member, ownerHeaders, connectMember, disconnectMember};
  document.addEventListener("DOMContentLoaded", mountHeader, {once: true});
  addEventListener("pageshow", event => { if (event.persisted) refreshAccess(true); });
  document.addEventListener("visibilitychange", () => { if (document.visibilityState === "visible") refreshAccess(true); });
  addEventListener("online", () => refreshAccess(true));
})();
