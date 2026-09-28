/* Aki is presentation only: never invent rewards, change study rules, or impersonate a friend. */
(() => {
  "use strict";
  const poses = new Set(["welcome", "review", "celebrate", "streak", "freeze", "winner", "face"]);
  const escape = value => String(value).replace(/[&<>"']/g, c => ({"&":"&amp;","<":"&lt;",">":"&gt;",'"':"&quot;","'":"&#39;"}[c]));
  const image = (pose = "welcome", extra = "") => `<img class="aki-art ${escape(extra)}" src="/aki/${poses.has(pose) ? pose : "welcome"}.png" width="128" height="128" alt="" aria-hidden="true" decoding="async" loading="lazy">`;
  function mood(profile) {
    if (profile?.day_ends_at > 0 && profile.day_ends_at <= Date.now()) return "review";
    if (profile?.streak_state === "protected" && !(profile?.today?.reviews > 0)) return "freeze";
    if (profile?.quests?.length && profile.quests.every(q => q.done === true)) return "celebrate";
    if (profile?.streak_state === "studied" && profile?.today?.reviews > 0) return "streak";
    return "review";
  }
  const messages = {
    welcome: "Aki is here to help. One card, one small step.",
    review: "A small step starts with one card.",
    celebrate: "Your daily quests are complete. Look at you go!",
    streak: "You showed up today. That is progress worth keeping.",
    freeze: "Your streak is protected by a freeze. A fresh start is waiting.",
  };
  function profile(value) {
    const pose = mood(value);
    return `<aside class="aki-companion" data-aki-companion="${pose}">${image(pose)}<p>${escape(AnkiQuestI18n.t(messages[pose]))}</p></aside>`;
  }
  function decorate(root = document) {
    const select = selector => [ ...(root.matches?.(selector) ? [root] : []), ...root.querySelectorAll?.(selector) || [] ];
    for (const brand of select(".brand img, .login-brand img")) {
      if (!brand.dataset.aki) { brand.src = "/aki/face.png"; brand.dataset.aki = "face"; }
    }
    for (const element of select(".page-hero:not([data-aki]), .empty:not([data-aki]), .loading-copy:not([data-aki]), .login-card:not([data-aki]), #view-trophies .card-head:not([data-aki]), #view-records .card-head:not([data-aki]), #winner-history .win-heading:not([data-aki])")) {
      let pose = "welcome";
      if (element.matches(".page-hero, #view-trophies .card-head, #view-records .card-head, #winner-history .win-heading")) pose = location.pathname === "/records" || element.closest("#view-trophies, #view-records, #winner-history") ? "winner" : "streak";
      if (element.matches(".loading-copy")) pose = "review";
      element.dataset.aki = pose;
      element.insertAdjacentHTML("afterbegin", image(pose));
    }
    // Dialog contents are recreated after close. Mark only the content, not the persistent dialog.
    for (const title of select("#streak-freezes h3:not([data-aki])")) {
      title.dataset.aki = "freeze";
      title.insertAdjacentHTML("beforebegin", image("freeze", "aki-dialog-art"));
    }
  }
  window.AnkiQuestAki = {image, mood, profile, decorate};
  document.addEventListener("DOMContentLoaded", () => {
    decorate();
    if (!document.body || typeof MutationObserver !== "function") return;
    new MutationObserver(records => {
      for (const record of records) for (const node of record.addedNodes) if (node.nodeType === 1) decorate(node);
    }).observe(document.body, {subtree:true, childList:true});
  }, {once:true});
})();
