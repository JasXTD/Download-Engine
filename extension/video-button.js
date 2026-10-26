/* Isolated-world UI. No page message bridge, network calls, cookies or tokens. */
(async () => {
  "use strict";
  await globalThis.IDMI18n?.ready;
  const __xdL = globalThis.IDMI18n || {t: text => text, f: (text, ...values) => text.replace(/\{(\d+)\}/g, (_, n) => String(values[n]))};
  if(globalThis.__idmVideoButton) { globalThis.__idmVideoButton.refresh(); return; }
  const api=globalThis.browser||globalThis.chrome;
  if(api.extension?.inIncognitoContext)return;
  let host,shadow,observer,performanceObserver,timer,stopped=false,enabled=false,refreshId=0,buttonWidth=0;
  const buttons=new Map(), encrypted=new WeakSet();
  const buttonHeight=36,maxButtonWidth=240;
  const labels={idle:"Download video",busy:"Opening…",ok:"Opened in app",err:"Try again"};
  const icons={idle:[["M12 4v10M8 10l4 4 4-4M6 19h12"]],busy:[["M12 4a8 8 0 1 1 0 16a8 8 0 1 1 0-16","track"],["M12 4a8 8 0 0 1 8 8"]],
    ok:[["M5.5 12.5l4 4 9-9"]],err:[["M19.5 12a7.5 7.5 0 1 1-2.2-5.3M19.5 4.5v4h-4"]]};
  // Every state keeps white text at 4.5:1 or more, so hover darkens rather than brightens.
  // The base background-color stays opaque because gradients cannot be transitioned.
  const buttonCss=`
    button {
      all:initial;position:fixed;box-sizing:border-box;pointer-events:auto;cursor:pointer;
      --top:#1873ea;--bottom:#0a5ad0;--edge:#084bb0;--glow:#0a5ad033;
      display:flex;align-items:center;justify-content:center;gap:7px;height:${buttonHeight}px;max-width:${maxButtonWidth}px;
      padding-block:0;padding-inline:10px 14px;border:1px solid var(--edge);border-radius:10px;
      background-color:var(--bottom);background-image:linear-gradient(180deg,var(--top),var(--bottom));
      color:#fff;font:600 13px/18px system-ui,sans-serif;white-space:nowrap;
      box-shadow:inset 0 1px 0 #ffffff33,0 1px 2px #00184a38,0 4px 12px var(--glow);
      transition:filter 140ms ease,box-shadow 140ms ease,transform 140ms ease;
    }
    button[data-state="ok"] {--top:#1d8c5f;--bottom:#116e4c;--edge:#0c5a3e;--glow:#116e4c33;}
    button[data-state="err"] {--top:#d9493f;--bottom:#b8352d;--edge:#982b24;--glow:#b8352d33;}
    button:hover:not(:disabled) {filter:brightness(.93);}
    button:active:not(:disabled) {filter:brightness(.86);box-shadow:inset 0 1px 3px #0000004d;transform:translateY(.5px);}
    button:focus-visible {outline:2px solid #fff;outline-offset:2px;box-shadow:0 0 0 6px #0a5ad0;}
    button:disabled {cursor:progress;}
    svg {display:block;flex:none;width:18px;height:18px;fill:none;stroke:currentColor;stroke-width:2.2;
      stroke-linecap:round;stroke-linejoin:round;pointer-events:none;}
    .track {opacity:.3;}
    button[data-state="busy"] svg {animation:spin 800ms linear infinite;}
    @keyframes spin {to {transform:rotate(360deg);}}
    .label {min-width:0;overflow:hidden;text-overflow:ellipsis;}
    @media (prefers-reduced-motion:reduce) {button {transition:none;} button[data-state="busy"] svg {animation-duration:2400ms;}}
  `;
  const style=(element,value)=>{element.style.cssText=value;};
  function icon(shapes) {
    const svg=document.createElementNS("http://www.w3.org/2000/svg","svg");
    svg.setAttribute("viewBox","0 0 24 24");svg.setAttribute("aria-hidden","true");
    svg.setAttribute("focusable","false");
    for(const [path,className] of shapes) {
      const shape=document.createElementNS("http://www.w3.org/2000/svg","path");
      shape.setAttribute("d",path);if(className)shape.setAttribute("class",className);svg.append(shape);
    }
    return svg;
  }
  function createButton() {
    const button=document.createElement("button"); button.type="button";
    button.lang=__xdL.language || "en";
    button.dir=__xdL.language === "ar" ? "rtl" : "ltr";
    const label=document.createElement("span");label.className="label";label.textContent=__xdL.t(labels.idle);
    label.setAttribute("aria-live","polite");
    button.append(icon(icons.idle),label);
    return {button,label};
  }
  function show(button,label,state) {
    button.setAttribute("data-state",state);
    button.firstChild.replaceWith(icon(icons[state]));
    label.textContent=__xdL.t(labels[state]);
  }
  // All buttons share the width of the widest state label in the current language, so a
  // button keeps its size and place when Opening…, Opened in app or Try again replaces the label.
  function measureWidth() {
    if(buttonWidth)return buttonWidth;
    const {button,label}=createButton();
    button.style.visibility="hidden"; shadow.append(button);
    let widest=0;
    for(const text of Object.values(labels)) {label.textContent=__xdL.t(text);widest=Math.max(widest,button.getBoundingClientRect().width);}
    button.remove();
    // A page that hides <html> measures 0. Place with the largest size and measure again next time.
    if(!(widest>0))return maxButtonWidth;
    return buttonWidth=Math.min(Math.ceil(widest),maxButtonWidth);
  }
  function cleanup() {
    enabled=false; observer?.disconnect(); performanceObserver?.disconnect(); clearTimeout(timer); timer=null;
    host?.remove(); host=null; buttons.clear(); buttonWidth=0;
    document.removeEventListener("scroll",schedule,true); window.removeEventListener("resize",schedule);
    document.removeEventListener("loadedmetadata",schedule,true); document.removeEventListener("encrypted",onEncrypted,true);
    document.removeEventListener("fullscreenchange",schedule);
  }
  function onEncrypted(event) { encrypted.add(event.target); schedule(); }
  // A click reviews this page's own address. YouTube, Bilibili and Twitch decorate their
  // other pages with muted preview players (carousel banners, hover previews), so there
  // only a video page's main player gets a button.
  function offered(video) {
    if(/(^|\.)youtube(-nocookie)?\.com$/.test(location.hostname))
      return /^\/(watch$|shorts\/|embed\/|live\/)/.test(location.pathname) && !!video.closest("#movie_player, #shorts-player");
    if(/^(www\.)?bilibili\.com$/.test(location.hostname))
      return /^\/(video\/(BV[0-9A-Za-z]{10}|av\d+)|bangumi\/play\/(ep|ss)\d+)\/?$/.test(location.pathname) && !!video.closest(".bpx-player-container");
    if(/^(www\.|m\.)?twitch\.tv$/.test(location.hostname))
      return /^\/videos\/\d+\/?$/.test(location.pathname) && !!video.closest('[data-a-target="video-player"]');
    if(/^(www\.|m\.)?tiktok\.com$/.test(location.hostname)) {
      // Video pages also preload the next video; only the page's own one gets a button.
      const id=location.pathname.match(/^\/@[\w.]*\/video\/(\d{1,25})\/?$/)?.[1];
      return !!id && !!video.closest(`[id^="xgwrapper-"][id$="-${id}"]`);
    }
    return true;
  }
  function schedule() { if(!enabled || timer)return; timer=setTimeout(()=>{timer=null;render();},150); }
  function placement(box,width,players,occupied) {
    const height=buttonHeight,gap=6,margin=4;
    const left=Math.max(margin,Math.min(box.right-width,innerWidth-width-margin));
    const top=Math.max(margin,Math.min(box.top,innerHeight-height-margin));
    const candidates=[{left,top:box.top-gap-height},{left,top:box.bottom+gap},
      {left:box.right+gap,top},{left:box.left-gap-width,top}];
    return candidates.map(p=>({...p,right:p.left+width,bottom:p.top+height})).find(p=>
      p.left>=margin && p.top>=margin && p.right<=innerWidth-margin && p.bottom<=innerHeight-margin &&
      [...players,...occupied].every(other=>p.right<=other.left || p.left>=other.right || p.bottom<=other.top || p.top>=other.bottom));
  }
  function render() {
    if(!enabled || !host)return;
    if(!host.isConnected) document.documentElement.append(host);
    const visible=new Set(),players=[],occupied=[];let width=0;
    for(const video of Array.from(document.querySelectorAll("video")).slice(0,100)) {
      const box=video.getBoundingClientRect(), css=getComputedStyle(video);
      if(box.width<180 || box.height<100 || box.bottom<=0 || box.top>=innerHeight || box.right<=0 || box.left>=innerWidth || css.visibility==="hidden" || css.display==="none")continue;
      players.push({video,box});
    }
    for(const {video,box} of players) {
      // Previews stay in players above, so buttons never cover them either.
      if(visible.size>=8 || video.mediaKeys || encrypted.has(video) || !offered(video))continue;
      if(!width)width=measureWidth();
      const position=placement(box,width,players.map(p=>p.box),occupied);
      if(!position)continue;
      occupied.push(position);
      visible.add(video);
      let button=buttons.get(video);
      if(!button) {
        const created=createButton(),label=created.label;button=created.button;
        button.title=__xdL.t("Choose video files and stream quality in Download Engine");
        button.setAttribute("aria-label",__xdL.t("Download video with Download Engine — review first"));
        button.addEventListener("click",async event=>{
          if(!event.isTrusted || !enabled || button.disabled)return;
          event.preventDefault(); event.stopPropagation(); button.disabled=true; show(button,label,"busy"); button.title=__xdL.t("Opening video review in Download Engine");
          button.setAttribute("aria-busy","true");
          try { const reply=await api.runtime.sendMessage({action:"on_page_videos"}); if(!reply?.ok)throw Error(reply?.message||__xdL.t("Open the Download Engine extension and try again.")); show(button,label,"ok"); button.title=__xdL.t("Review the video in Download Engine"); }
          catch(error) {show(button,label,"err");button.title=error.message;}
          finally {button.disabled=false;button.setAttribute("aria-busy","false");}
        });
        shadow.append(button); buttons.set(video,button);
      }
      button.style.width=`${width}px`;
      button.style.left=`${position.left}px`;
      button.style.top=`${position.top}px`;
    }
    for(const [video,button] of buttons) if(!visible.has(video)) {button.remove();buttons.delete(video);}
  }
  async function refresh() {
    const current=++refreshId;
    try {
      const reply=await api.runtime.sendMessage({action:"video_button_state"});
      if(current!==refreshId)return;
      if(stopped || !reply?.ok || reply.enabled!==true) {cleanup();return;}
      if(enabled) {schedule();return;}
      enabled=true; host=document.createElement("div"); host.setAttribute("data-idm-ex-video-button","");
      style(host,"all:initial!important;position:fixed!important;top:0!important;left:0!important;width:0!important;height:0!important;z-index:2147483647!important;pointer-events:none!important;");
      shadow=host.attachShadow({mode:"closed"});
      const stylesheet=document.createElement("style");stylesheet.textContent=buttonCss;shadow.append(stylesheet);
      document.documentElement.append(host);
      observer=new MutationObserver(schedule); observer.observe(document.documentElement,{childList:true,subtree:true,attributes:true,attributeFilter:["src","type","class","style"]});
      document.addEventListener("scroll",schedule,true);window.addEventListener("resize",schedule);
      document.addEventListener("loadedmetadata",schedule,true); document.addEventListener("encrypted",onEncrypted,true);
      document.addEventListener("fullscreenchange",schedule);
      if(typeof PerformanceObserver!=="undefined") {performanceObserver=new PerformanceObserver(schedule);performanceObserver.observe({type:"resource",buffered:true});}
      render();
    } catch { if(current===refreshId)cleanup(); }
  }
  api.storage.onChanged.addListener((changes,area)=>{if(area==="local" && (changes.videoButtonDisabledSites || changes.videoButtonPermissions))void refresh();});
  window.addEventListener("xd-language-changed",()=>{
    // Recreate only this extension's controls. Leave the page and playback alone.
    for(const button of buttons.values()) button.remove();
    buttons.clear(); buttonWidth=0; schedule();
  });
  window.addEventListener("pagehide",()=>{stopped=true;cleanup();});
  window.addEventListener("pageshow",()=>{stopped=false;void refresh();});
  globalThis.__idmVideoButton={refresh}; void refresh();
})();
