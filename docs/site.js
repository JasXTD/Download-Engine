"use strict";

const root = document.documentElement;
const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)");
root.classList.add("js");

// Scroll reveal. Content stays visible without JavaScript, without
// IntersectionObserver and whenever reduced motion is requested.
(() => {
  const targets = [...document.querySelectorAll(".reveal")];
  if (reducedMotion.matches || !("IntersectionObserver" in window)) return;

  // Anything already on screen is left alone so the first paint never flickers.
  const viewportHeight = window.innerHeight;
  const pending = targets.filter(target => {
    const box = target.getBoundingClientRect();
    if (box.top < viewportHeight && box.bottom > 0) {
      target.classList.remove("reveal");
      return false;
    }
    return true;
  });
  const observer = new IntersectionObserver(entries => {
    for (const entry of entries) {
      if (!entry.isIntersecting) continue;
      entry.target.classList.add("is-visible");
      observer.unobserve(entry.target);
    }
  }, { rootMargin: "0px 0px -10% 0px", threshold: 0 });
  pending.forEach(target => observer.observe(target));
  root.classList.add("motion");

  reducedMotion.addEventListener("change", () => {
    if (!reducedMotion.matches) return;
    observer.disconnect();
    root.classList.remove("motion");
  });
})();

// Header shadow once the page scrolls, and the current section in the main navigation.
(() => {
  const header = document.querySelector("[data-site-header]");
  if (header) {
    let queued = false;
    const update = () => {
      queued = false;
      header.classList.toggle("is-scrolled", window.scrollY > 8);
    };
    window.addEventListener("scroll", () => {
      if (queued) return;
      queued = true;
      window.requestAnimationFrame(update);
    }, { passive: true });
    update();
  }

  const links = [...document.querySelectorAll('.site-nav a[href^="#"]')];
  const sections = links.map(link => document.getElementById(link.hash.slice(1))).filter(Boolean);
  if (!sections.length || !("IntersectionObserver" in window)) return;
  const inView = new Set();
  const observer = new IntersectionObserver(entries => {
    for (const entry of entries) {
      if (entry.isIntersecting) inView.add(entry.target);
      else inView.delete(entry.target);
    }
    const current = sections.find(section => inView.has(section));
    for (const link of links) {
      if (current && link.hash === `#${current.id}`) link.setAttribute("aria-current", "true");
      else link.removeAttribute("aria-current");
    }
  }, { rootMargin: "-45% 0px -50% 0px" });
  sections.forEach(section => observer.observe(section));
})();

// Hero theme stack: rotates the featured themes while visible. Hover and keyboard
// focus suspend rotation; choosing a theme or Pause stops it until Play is chosen.
for (const stack of document.querySelectorAll("[data-theme-stack]")) {
  const hero = stack.closest(".hero") || root;
  const frames = [...stack.querySelectorAll(".stack-frame")];
  const controls = stack.querySelector(".stack-controls");
  const chips = [...stack.querySelectorAll(".stack-chip")];
  const toggle = stack.querySelector(".stack-toggle");
  const toggleLabel = stack.querySelector("[data-stack-toggle-label]");
  const name = stack.querySelector("[data-stack-name]");
  const fullSize = stack.querySelector("[data-stack-full]");
  let current = 0;
  let playing = !reducedMotion.matches;
  let visible = !("IntersectionObserver" in window);
  let hovered = false;
  let focused = false;
  let active = true;
  let timer;
  let revision = 0;

  const canAdvance = () => playing && visible && !hovered && !focused && active && !document.hidden;

  function schedule() {
    window.clearTimeout(timer);
    timer = canAdvance() ? window.setTimeout(() => show(current + 1, false), 3800) : undefined;
  }

  function updateToggle() {
    toggleLabel.textContent = playing ? "Pause" : "Play";
    toggle.setAttribute("aria-label", playing ? "Pause theme preview" : "Play theme preview");
  }

  async function show(index, manual) {
    window.clearTimeout(timer);
    if (manual) {
      playing = false;
      updateToggle();
    }
    const request = ++revision;
    const next = (index + frames.length) % frames.length;
    const image = frames[next].querySelector("img");
    try {
      await image.decode();
    } catch (error) {
      if (request !== revision) return;
      playing = false;
      updateToggle();
      console.warn("Theme preview image could not load", error);
      return;
    }
    // A newer choice or a pause while decoding supersedes this change.
    if (request !== revision || !active) return;
    if (!manual && !canAdvance()) { schedule(); return; }
    current = next;
    frames.forEach((frame, position) => {
      const depth = (position - current + frames.length) % frames.length;
      frame.dataset.pos = String(depth);
      if (depth === 0) frame.removeAttribute("aria-hidden");
      else frame.setAttribute("aria-hidden", "true");
    });
    chips.forEach((chip, position) => chip.setAttribute("aria-pressed", String(position === current)));
    const theme = frames[current].dataset.theme;
    name.textContent = theme;
    fullSize.href = image.getAttribute("src");
    fullSize.setAttribute("aria-label", `Open the ${theme} theme screenshot at full size in a new tab`);
    hero.style.setProperty("--hero-glow", frames[current].dataset.glow);
    schedule();
  }

  chips.forEach((chip, index) => chip.addEventListener("click", () => show(index, true)));
  // The dimmed frames peeking out behind the front screenshot are pointer shortcuts;
  // keyboard users have the equivalent buttons.
  frames.forEach((frame, index) => frame.addEventListener("click", () => {
    if (frame.dataset.pos !== "0") show(index, true);
  }));
  toggle.addEventListener("click", () => {
    ++revision;
    playing = !playing;
    updateToggle();
    schedule();
  });
  stack.addEventListener("mouseenter", () => { hovered = true; ++revision; schedule(); });
  stack.addEventListener("mouseleave", () => { hovered = false; schedule(); });
  // Only keyboard focus suspends rotation; a clicked Play button keeps focus but should play.
  stack.addEventListener("focusin", event => {
    focused = event.target.matches(":focus-visible");
    if (focused) ++revision;
    schedule();
  });
  stack.addEventListener("focusout", event => {
    if (stack.contains(event.relatedTarget)) return;
    focused = false;
    schedule();
  });
  reducedMotion.addEventListener("change", () => {
    // Switching out of reduced motion never overrides a deliberate pause.
    if (reducedMotion.matches) { ++revision; playing = false; updateToggle(); }
    schedule();
  });
  document.addEventListener("visibilitychange", () => { ++revision; schedule(); });
  window.addEventListener("pagehide", () => { ++revision; active = false; window.clearTimeout(timer); });
  window.addEventListener("pageshow", () => { active = true; schedule(); });

  if (!visible) {
    new IntersectionObserver(([entry]) => {
      visible = entry.isIntersecting && entry.intersectionRatio >= 0.25;
      if (!visible) ++revision;
      schedule();
    }, { threshold: [0, 0.25] }).observe(stack);
  }
  updateToggle();
  controls.hidden = false;
  schedule();
}
