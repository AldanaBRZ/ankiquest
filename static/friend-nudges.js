/* Friend nudges use the same private account as the community page. */
(() => {
  const plain = (source, ...values) => Array.isArray(source) ? source.reduce((text, part, index) => text + (index ? values[index - 1] : "") + part, "") : source;
  const aqText = (...args) => (window.AnkiQuestI18n?.t || plain)(...args);
  const aqHtml = (...args) => (window.AnkiQuestI18n?.html || plain)(...args);
  const dialog = document.createElement("dialog");
  dialog.className = "dialog";
  dialog.setAttribute("aria-label", aqText("Nudge your friends"));
  document.body.append(dialog);
  let session = null, busy = false, generation = 0;
  const close = () => { generation++; session = null; busy = false; dialog.close(); dialog.replaceChildren(); };
  const current = (captured, epoch) => session === captured && state.session === captured && generation === epoch && dialog.open;
  function render(payload) {
    dialog.innerHTML = aqHtml`
      <div class="card-head"><h2>Nudge your friends</h2><button type="button" data-nudge-close aria-label="Close">Close</button></div>
      <p>A little encouragement to do Anki. One nudge per friend per Anki day.</p>
      <label class="check"><input type="checkbox" data-nudge-receiving ${payload.receiving ? "checked" : ""}><span>Let friends send me nudges</span></label>
      <label class="check"><input type="checkbox" data-nudge-automatic ${payload.automatic_receiving ? "checked" : ""}><span>Let AnkiQuest send me progress nudges</span></label>
      <p class="settings-hint">Choose which friends can nudge you below. Phone vibrations follow your alert settings, silent mode, and Do Not Disturb.</p>
      <div class="stack">${payload.friends.map(friend => aqHtml`
        <div class="list-row">${avatar(friend.user, friend.display)}<div class="row-text">
          <strong>${esc(friend.display)}</strong>
          <small>${friend.sent_today ? aqText("Nudged today") : friend.enabled ? aqText("Ready for encouragement") : aqText("Not receiving nudges")}</small>
          <label class="check"><input type="checkbox" data-nudge-sender="${esc(friend.user)}" aria-label="${esc(aqText("Receive nudges from") + " " + friend.display)}" ${friend.muted_by_me ? "" : "checked"}><span>Receive their nudges</span></label>
        </div><button type="button" data-nudge-user="${esc(friend.user)}" ${!friend.enabled || friend.sent_today ? "disabled" : ""}>Nudge</button></div>`).join("") || aqHtml(["<p>No friends are available yet.</p>"])}</div>
      <p data-nudge-status role="status"></p>`;
  }
  async function open() {
    if (!state.session) return;
    close(); session = state.session;
    const captured = session, epoch = ++generation;
    dialog.innerHTML = aqHtml(['<p role="status">Loading friends…</p><button type="button" data-nudge-close>Close</button>']);
    dialog.showModal();
    try {
      const payload = await ownerRequest(`/api/friend-nudges/${encodeURIComponent(captured.user)}`, undefined, captured);
      if (current(captured, epoch)) render(payload);
    } catch (error) {
      if (current(captured, epoch)) dialog.innerHTML = aqHtml`<p role="alert">${esc(error.status === 404 ? aqText("Update the server to use friend nudges.") : error.message)}</p><button type="button" data-nudge-close>Close</button>`;
    }
  }
  async function change(control, mode) {
    if (busy || !session || state.session !== session) return;
    const captured = session, epoch = generation, wanted = control.checked;
    busy = true;
    const controls = [...dialog.querySelectorAll("input,button:not([data-nudge-close])")];
    const disabled = controls.map(item => item.disabled);
    controls.forEach(item => item.disabled = true);
    const status = dialog.querySelector("[data-nudge-status]"); status.textContent = mode === "send" ? aqText("Sending…") : aqText("Saving…");
    try {
      const base = `/api/friend-nudges/${encodeURIComponent(captured.user)}`;
      const path = mode === "friend" ? "/receiving" : mode === "automatic" ? "/automatic" : mode === "sender" ? `/senders/${encodeURIComponent(control.dataset.nudgeSender)}` : "";
      await ownerRequest(base + path, mode === "send" ? {recipient:control.dataset.nudgeUser} : {enabled:wanted}, captured);
      if (!current(captured, epoch)) return;
      const payload = await ownerRequest(`/api/friend-nudges/${encodeURIComponent(captured.user)}`, undefined, captured);
      if (current(captured, epoch)) { render(payload); dialog.querySelector("[data-nudge-status]").textContent = mode === "send" ? aqText("Nudge sent!") : aqText("Preference saved."); }
    } catch (error) {
      if (current(captured, epoch)) {
        if (mode !== "send") control.checked = !wanted;
        controls.forEach((item, index) => item.disabled = disabled[index]);
        status.textContent = error.status === 409 ? aqText("You already nudged this friend today.") : error.status === 403 && mode === "send" ? aqText("This friend is not receiving nudges.") : "Could not confirm the change. Refresh and try again. " + error.message;
      }
    } finally { if (current(captured, epoch)) busy = false; }
  }
  document.addEventListener("click", event => {
    const button = event.target.closest("button"); if (!button) return;
    if (button.hasAttribute("data-nudge-friends")) open();
    if (button.hasAttribute("data-nudge-close") || button.hasAttribute("data-disconnect")) close();
    if (button.dataset.nudgeUser) change(button, "send");
  });
  dialog.addEventListener("change", event => {
    if (event.target.hasAttribute("data-nudge-receiving")) change(event.target, "friend");
    if (event.target.hasAttribute("data-nudge-automatic")) change(event.target, "automatic");
    if (event.target.hasAttribute("data-nudge-sender")) change(event.target, "sender");
  });
  dialog.addEventListener("cancel", close);
  addEventListener("ankiquest:locked", close);
  addEventListener("ankiquest-auth", close);
  addEventListener("pagehide", close);
})();
