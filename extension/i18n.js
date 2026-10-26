/* Display text only. Commands, permission names, URLs and saved identifiers are never translated. */
(() => {
  "use strict";
  if (globalThis.IDMI18n) return;
  const api = globalThis.browser || globalThis.chrome;
  const choices = ["system", "en", "zh-CN", "zh-TW", "ms", "ja", "ko", "vi", "th", "pl", "ru", "uk", "ar", "de", "fr", "sv", "no", "de-CH", "it", "rm"];
  let language = "en";
  function resolve(value) {
    const parts = String(value || "en").toLowerCase().replaceAll("_", "-").split("-");
    if (parts[0] === "zh") return parts.includes("hant") || (!parts.includes("hans") && parts.some(p => ["tw", "hk", "mo"].includes(p))) ? "zh-TW" : "zh-CN";
    if (parts[0] === "de" && parts.includes("ch")) return "de-CH";
    // Chromium uses no; Windows and Firefox also identify Bokmål as nb.
    if (parts[0] === "nb") return "no";
    return choices.includes(parts[0]) && parts[0] !== "system" ? parts[0] : "en";
  }
  function t(text) {
    return globalThis.IDMLanguageCatalog?.[text]?.[language] ?? text;
  }
  function f(text, ...values) {
    return t(text).replace(/\{(\d+)\}/g, (placeholder, index) => index < values.length ? String(values[index]) : placeholder);
  }
  function translateDocument() {
    if (typeof document === "undefined") return;
    document.documentElement.lang = language;
    document.documentElement.dir = language === "ar" ? "rtl" : "ltr";
    // Only initial, authored HTML is visited. Never watch user/page/diagnostic data
    // with a MutationObserver: an actual filename may equal an English UI label.
    const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
    while (walker.nextNode()) {
      const node = walker.currentNode;
      if (node.parentElement.closest("script, style, textarea, pre, code")) continue;
      const key = node.textContent.trim();
      if (key) node.textContent = node.textContent.replace(key, t(key));
    }
    for (const element of document.querySelectorAll("[aria-label], [title], [placeholder]")) {
      for (const attr of ["aria-label", "title", "placeholder"]) if (element.hasAttribute(attr)) element.setAttribute(attr, t(element.getAttribute(attr)));
    }
    document.title = t(document.title);
  }
  function apply(value) {
    language = value && value !== "system" && choices.includes(value) ? value : resolve(api?.i18n?.getUILanguage?.() || globalThis.navigator?.language);
  }
  apply("system");
  const ready = (async () => {
    try { const saved = await api?.storage?.local?.get("displayLanguage"); apply(saved?.displayLanguage); }
    catch (error) { console.warn("Cannot load display language", error); }
  })();
  api?.storage?.onChanged?.addListener((changes, area) => {
    if (area === "local" && changes.displayLanguage) {
      apply(changes.displayLanguage.newValue);
      globalThis.dispatchEvent?.(new Event("xd-language-changed"));
    }
  });
  async function setLanguage(value) {
    if (!choices.includes(value)) throw Error("Invalid display language");
    await api.storage.local.set({displayLanguage: value});
    apply(value);
  }
  globalThis.IDMI18n = {t, f, ready, resolve, translateDocument, setLanguage, choices: Object.freeze(choices), get language() { return language; }};
  if (typeof module !== "undefined") module.exports = globalThis.IDMI18n;
})();
