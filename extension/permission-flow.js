/* Permission prompts can destroy their popup. Keep completion and the optional
   return to that popup in the background, tied to one explicit setting change. */
(function (root) {
  "use strict";
  const __xdL = globalThis.IDMI18n || {t: text => text, f: (text, ...values) => text.replace(/\{(\d+)\}/g, (_, n) => String(values[n]))};
  const RETURN_PREFIX = "permission-popup-return:";
  class Manager {
    // Window timers require their Window receiver in Firefox. Wrappers prevent
    // calls such as this.schedule(...) from rebinding that receiver to Manager.
    constructor({ api, settings, report, schedule = (fn, delay) => root.setTimeout(fn, delay), unschedule = id => root.clearTimeout(id) }) {
      Object.assign(this, { api, settings, report, schedule, unschedule });
      this.pending = null;
    }
    cancel(setting) {
      if (this.pending && (!setting || this.pending.setting === setting)) {
        const pending = this.pending;
        pending.cancelled = true;
        pending.stopWaiting?.();
        this.pending = null;
      }
    }
    popupOpened(target) {
      if (this.pending?.phase === "asking" && this.pending.target.windowId === target.windowId) this.cancel();
    }
    tabLeft(tabId) { if (this.pending?.phase === "asking" && this.pending.target.tabId === tabId) this.cancel(); }
    request(message, requestInPopup) {
      // Firefox can call this function across extension windows. Do not retain
      // objects owned by a popup that the permission prompt is about to destroy.
      message = structuredClone(message);
      const setting = this.settings[message.setting];
      if (!Object.hasOwn(this.settings, message.setting) || !setting) throw Error(__xdL.t("Unknown permission setting."));
      if (this.pending) throw Error(__xdL.t("Finish the open permission request before changing another setting."));
      const grant = setting.grant(message);
      const target = message.returnTo;
      if (!target || !Number.isInteger(target.windowId) || target.windowId < 0 ||
          !Number.isInteger(target.tabId) || target.tabId < 0) throw Error(__xdL.t("Reopen the extension and try again."));
      const pending = { setting: message.setting, target: { windowId: target.windowId, tabId: target.tabId }, cancelled: false, phase: "asking" };
      this.pending = pending;
      let before, requested;
      try {
        before = this.api.permissions.contains(grant);
        // Firefox must attach its prompt to the visible popup. Its promise can
        // disappear with that document, so the background also watches the grant.
        // Chromium forwards the popup gesture to its background message listener.
        if (requestInPopup) {
          let complete, timer;
          const added = () => {
            void this.api.permissions.contains(grant).then(allowed => { if (allowed) complete(true); }, () => complete(false));
          };
          const observed = new Promise(resolve => { complete = resolve; });
          const cleanup = () => {
            this.api.permissions.onAdded.removeListener(added);
            if (timer !== undefined) { this.unschedule(timer); timer = undefined; }
          };
          pending.cleanup = cleanup;
          pending.stopWaiting = () => { cleanup(); complete(false); };
          this.api.permissions.onAdded.addListener(added);
          // A dismissed prompt has no rejection event after Firefox destroys the
          // popup. Bound that abandoned intent; reopening the popup or leaving
          // its tab cancels it immediately, before any later permission grant.
          timer = this.schedule(() => { if (this.pending === pending && pending.phase === "asking") this.cancel(pending.setting); }, 5 * 60 * 1000);
          requested = Promise.race([requestInPopup(grant), observed]);
        } else requested = this.api.permissions.request(grant);
      } catch (error) {
        requested = Promise.reject(error);
      }
      return this.finish(pending, setting, message, before, requested);
    }
    async finish(pending, setting, message, before, requested) {
      let granted = false, wasAllowed = true, reply;
      try {
        try {
          [wasAllowed, granted] = await Promise.all([before, requested]);
          pending.cleanup?.();
          pending.phase = "finishing";
          if (pending.cancelled) return { ok: false, message: __xdL.t("The setting changed while permission was being requested.") };
          reply = granted ? await setting.configure(message) : { ok: false, message: setting.denied };
        } catch (error) {
          reply = { ok: false, message: error.message };
        }
        if (pending.cancelled) return { ok: false, message: __xdL.t("The setting changed while permission was being requested.") };
        await this.report(reply.ok, reply.message);
        if (granted && !wasAllowed) await this.restore(pending, reply);
        return reply;
      } finally {
        pending.cleanup?.();
        if (this.pending === pending) this.pending = null;
      }
    }
    async restore(pending, reply) {
      const { windowId, tabId } = pending.target;
      const key = RETURN_PREFIX + windowId;
      try {
        const window = await this.api.windows.get(windowId).catch(() => null);
        if (pending.cancelled || !window?.focused) return;
        const [tab] = await this.api.tabs.query({ active: true, windowId });
        if (pending.cancelled || tab?.id !== tabId) return;
        // Chromium popup contexts report windowId -1. Filtering them by the
        // browser window would miss a popup that is already open.
        const views = this.api.runtime.getContexts
          ? await this.api.runtime.getContexts({ contextTypes: ["POPUP"] })
          : this.api.extension.getViews({ type: "popup", windowId });
        if (views.length) return;
        await this.api.storage.session.set({ [key]: { setting: pending.setting, tabId } });
        if (pending.cancelled) { await this.api.storage.session.remove(key); return; }
        await (this.api.action || this.api.browserAction).openPopup({ windowId });
      } catch (error) {
        await this.api.storage.session.remove(key);
        // Older Firefox releases can require another user gesture to open UI.
        // The approved setting is already saved; only the automatic return failed.
        console.warn(__xdL.t("Could not return to the extension popup:"), error);
        await this.report(reply.ok, reply.message + " Reopen Download Engine from the toolbar to continue.");
      }
    }
  }
  root.IDMPermissionFlow = { Manager, RETURN_PREFIX };
})(globalThis);
