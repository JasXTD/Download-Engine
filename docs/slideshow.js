"use strict";

for (const carousel of document.querySelectorAll("[data-slideshow]")) {
  const slides = [...carousel.querySelectorAll(".screenshot-slide")];
  const controls = carousel.querySelector(".slideshow-controls");
  const toggle = carousel.querySelector(".slideshow-toggle");
  const toggleLabel = carousel.querySelector("[data-slideshow-toggle-label]");
  const dots = [...carousel.querySelectorAll(".slideshow-dots button")];
  const track = carousel.querySelector(".slideshow-track");
  const count = carousel.querySelector(".slideshow-count");
  const message = carousel.querySelector(".slideshow-message");
  const motion = window.matchMedia("(prefers-reduced-motion: reduce)");
  let current = 0;
  let playing = !motion.matches;
  let visible = false;
  let hovered = carousel.matches(":hover");
  let active = true;
  let timer;
  let revision = 0;
  let pointerPlayback;

  function canAdvance() {
    return playing && visible && !hovered && active && !document.hidden;
  }

  function clearTimer() {
    window.clearTimeout(timer);
    timer = undefined;
  }

  function schedule() {
    clearTimer();
    if (canAdvance()) timer = window.setTimeout(() => show(current + 1, false), 2000);
  }

  function updateControls() {
    toggleLabel.textContent = playing ? "Pause" : "Play";
    toggle.setAttribute("aria-label", playing ? "Pause slideshow" : "Play slideshow");
    // Automatic changes must not repeatedly interrupt screen-reader speech.
    track.setAttribute("aria-live", playing ? "off" : "polite");
  }

  async function show(index, manual) {
    clearTimer();
    if (manual) playing = false;
    updateControls();
    const request = ++revision;
    const next = (index + slides.length) % slides.length;
    const image = slides[next].querySelector("img");
    image.loading = "eager";
    try {
      await image.decode();
    } catch (error) {
      if (request !== revision || !active) return;
      playing = false;
      updateControls();
      message.textContent = "This screenshot could not load. Choose another image or try again.";
      message.hidden = false;
      console.warn("Slideshow image could not load", error);
      return;
    }
    // A newer choice or a page lifecycle event supersedes pending decoding.
    if (request !== revision || !active) return;
    if (!manual && !canAdvance()) { schedule(); return; }
    slides.forEach((slide, position) => { slide.hidden = position !== next; });
    dots.forEach((dot, position) => dot.setAttribute("aria-pressed", String(position === next)));
    current = next;
    count.textContent = `${current + 1} / ${slides.length}`;
    message.hidden = true;
    schedule();
  }

  // Pointer focus pauses first; retain the intent of the label that was clicked.
  toggle.addEventListener("pointerdown", () => { pointerPlayback = playing; });
  toggle.addEventListener("pointercancel", () => { pointerPlayback = undefined; });
  toggle.addEventListener("click", event => {
    ++revision;
    const previous = event.detail > 0 && pointerPlayback !== undefined ? pointerPlayback : playing;
    pointerPlayback = undefined;
    playing = !previous;
    updateControls();
    schedule();
  });
  carousel.querySelector(".slideshow-previous").addEventListener("click", () => show(current - 1, true));
  carousel.querySelector(".slideshow-next").addEventListener("click", () => show(current + 1, true));
  dots.forEach((dot, index) => dot.addEventListener("click", () => show(index, true)));
  carousel.addEventListener("mouseenter", () => { hovered = true; ++revision; schedule(); });
  carousel.addEventListener("mouseleave", () => { hovered = false; schedule(); });
  carousel.addEventListener("focusin", () => {
    ++revision;
    playing = false;
    updateControls();
    schedule();
  });
  motion.addEventListener("change", () => {
    // Switching out of reduced motion never overrides a deliberate pause.
    if (motion.matches) { ++revision; playing = false; updateControls(); }
    schedule();
  });
  document.addEventListener("visibilitychange", () => { ++revision; schedule(); });
  window.addEventListener("pagehide", () => { ++revision; active = false; clearTimer(); });
  window.addEventListener("pageshow", () => { active = true; schedule(); });

  if ("IntersectionObserver" in window) {
    const observer = new IntersectionObserver(([entry]) => {
      visible = entry.isIntersecting && entry.intersectionRatio >= 0.25;
      if (!visible) ++revision;
      if (visible) {
        // Decode before showing a slide; eager loading here prevents a blank frame.
        for (const slide of slides) slide.querySelector("img").loading = "eager";
      }
      schedule();
    }, { threshold: [0, 0.25] });
    observer.observe(carousel);
  } else {
    visible = true;
  }
  updateControls();
  controls.hidden = false;
  schedule();
}
