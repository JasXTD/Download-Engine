/* Default-on video buttons within granted website access, with per-site opt-outs. Content messages only request
   desktop review; URLs are read again by our isolated collector. */
(function(root) {
  "use strict";
  const __xdL = globalThis.IDMI18n || {t: text => text, f: (text, ...values) => text.replace(/\{(\d+)\}/g, (_, n) => String(values[n]))};
  const ID = "idm-ex-video-button";
  function site(url) {
    const parsed = new URL(IDMCore.normalizeUrl(url));
    return `${parsed.protocol}//${parsed.hostname}/*`;
  }
  class Manager {
    constructor({api, pageLinks}) { this.api=api; this.pageLinks=pageLinks; this.tail=Promise.resolve(); this.last=new Map(); }
    async disabledSites() { const {videoButtonDisabledSites} = await this.api.storage.local.get("videoButtonDisabledSites"); return Array.isArray(videoButtonDisabledSites) ? [...new Set(videoButtonDisabledSites.filter(v=>{try{return typeof v==="string" && v===site(v);}catch{return false;}}))] : []; }
    async active(url) {
      const pattern=site(url);
      return !(await this.disabledSites()).includes(pattern) && await this.api.permissions.contains({origins:[pattern]});
    }
    async pageStatus(sender) {
      const enabled=sender.frameId===0 && Number.isInteger(sender.tab?.id) && !sender.tab.incognito && await this.active(sender.url);
      return {ok:true,enabled};
    }
    async current() {
      const tab=(await this.api.tabs.query({active:true,currentWindow:true}))[0];
      if (!tab || tab.incognito || !Number.isInteger(tab.id)) throw Error(__xdL.t("Open a regular web page to configure its video button."));
      return {tab,pattern:site(tab.url)};
    }
    async status() {
      const {pattern}=await this.current();
      return {ok:true,pattern,enabled:!(await this.disabledSites()).includes(pattern),permitted:await this.api.permissions.contains({origins:[pattern]})};
    }
    async syncNow(inject=false) {
        const disabled=await this.disabledSites();
        const {origins=[]}=await this.api.permissions.getAll();
        const matches=[...new Set(origins.flatMap(pattern=>pattern==='<all_urls>'?['https://*/*','http://*/*']:/^(https?|\*):\/\//.test(pattern)?[pattern]:[]))].filter(pattern=>!disabled.includes(pattern)).sort();
        // Notify already-injected pages when access changes, including revocation.
        // The background rechecks browser permissions before they render or hand off.
        await this.api.storage.local.set({videoButtonPermissions:matches});
        const registered=await this.api.scripting.getRegisteredContentScripts({ids:[ID]});
        if(registered.length) await this.api.scripting.unregisterContentScripts({ids:[ID]});
        if(matches.length) await this.api.scripting.registerContentScripts([{id:ID,matches,excludeMatches:disabled,js:["i18n-catalog.js","i18n.js","video-button.js"],runAt:"document_idle",allFrames:false,persistAcrossSessions:true}]);
        if(inject && matches.length) {
          // Register future pages and update already-open players without a reload.
          const tabs=[];
          for(const tab of await this.api.tabs.query({})) {
            if(tab.incognito || !Number.isInteger(tab.id) || !/^https?:\/\//.test(tab.url||""))continue;
            try { site(tab.url); } catch { continue; } // Skip unsupported/credential-bearing URLs.
            if(await this.active(tab.url)) tabs.push(tab);
          }
          const results=await Promise.allSettled(tabs.map(tab=>this.api.scripting.executeScript({target:{tabId:tab.id,frameIds:[0]},files:["i18n-catalog.js","i18n.js","video-button.js"]})));
          // Tabs can close or become browser-restricted between query and injection.
          for(const result of results) if(result.status==="rejected") console.warn(__xdL.t("Video button could not update an open tab:"),result.reason);
        }
    }
    schedule(run) { this.tail=this.tail.catch(()=>{}).then(run); return this.tail; }
    sync(inject=false) { return this.schedule(()=>this.syncNow(inject)); }
    configure(enabled,pattern) { return this.schedule(()=>this.configureNow(enabled,pattern)); }
    async configureNow(enabled,pattern) {
      if(typeof enabled!=="boolean") throw Error(__xdL.t("Invalid video button setting."));
      const current=await this.current();
      if(current.pattern!==pattern) throw Error(__xdL.t("The page changed. Reopen the extension and try again."));
      if(enabled && !await this.api.permissions.contains({origins:[pattern]})) throw Error(__xdL.t("Allow this site's access to show its video button."));
      const disabled=await this.disabledSites();
      if(!enabled && !disabled.includes(pattern)) {
        if(disabled.length>=1000) throw Error(__xdL.t("The saved video-button site limit has been reached."));
        disabled.push(pattern);
      }
      await this.api.storage.local.set({videoButtonDisabledSites:enabled?disabled.filter(v=>v!==pattern):disabled});
      await this.syncNow();
      if(enabled) await this.api.scripting.executeScript({target:{tabId:current.tab.id,frameIds:[0]},files:["i18n-catalog.js","i18n.js","video-button.js"]});
      return {ok:true,message:enabled?__xdL.t("Video buttons are on for this site. Click a button to review available videos."):__xdL.t("Video buttons are off for this site.")};
    }
    async open(sender) {
      if(sender.frameId!==0 || !Number.isInteger(sender.tab?.id) || sender.tab.incognito) throw Error(__xdL.t("Video buttons are available in regular top-level pages only."));
      if(!await this.active(sender.url)) throw Error(__xdL.t("Video buttons are disabled for this site or website access is missing."));
      const current=await this.api.tabs.get(sender.tab.id);
      // Chromium keeps the original sender URL after history.pushState (YouTube).
      // The collector verifies the active document ID before any native handoff.
      // Older browsers without document IDs retain the strict URL check.
      const documentId=typeof sender.documentId==="string" && sender.documentId ? sender.documentId : null;
      if(current.incognito || new URL(current.url).origin!==new URL(sender.url).origin ||
         (sender.documentLifecycle && sender.documentLifecycle!=="active") ||
         (!documentId && IDMCore.normalizeUrl(current.url)!==IDMCore.normalizeUrl(sender.url))) throw Error(__xdL.t("The page changed. Click its video button again."));
      if(Date.now()-(this.last.get(current.id)||0)<1500) throw Error(__xdL.t("A video review is already opening."));
      this.last.set(current.id,Date.now());
      return this.pageLinks.scan(current,"videos",documentId);
    }
  }
  root.IDMVideoButtons={Manager,site};
})(globalThis);
