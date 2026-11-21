(() => {
  "use strict";

  const $ = (sel, root = document) => root.querySelector(sel);
  const $$ = (sel, root = document) => [...root.querySelectorAll(sel)];

  // ---------------------------------------------------------- filter/search
  const input = $("#filter");
  const counter = $("#filter-count");
  const cards = $$(".msg");

  if (input && cards.length) {
    const searchable = new Map(
      cards.map((card) => [card, card.textContent.toLowerCase()])
    );

    const apply = () => {
      const query = input.value.trim().toLowerCase();
      let shown = 0;
      for (const card of cards) {
        const hit = !query || searchable.get(card).includes(query);
        card.hidden = !hit;
        if (hit) shown += 1;
      }
      if (counter) {
        counter.textContent = query
          ? `${shown} / ${cards.length}`
          : `${cards.length} messages`;
      }
    };

    input.addEventListener("input", apply);

    window.addEventListener("keydown", (event) => {
      const active = document.activeElement;
      if (event.key === "/" && active !== input) {
        event.preventDefault();
        input.focus();
        input.select();
      } else if (event.key === "Escape" && active === input) {
        input.value = "";
        apply();
        input.blur();
      }
    });
  }

  // ------------------------------------------------------- expand/collapse
  const setAll = (open) => {
    for (const detail of $$("details.block")) {
      detail.open = open;
    }
  };

  $("[data-act='expand']")?.addEventListener("click", () => setAll(true));
  $("[data-act='collapse']")?.addEventListener("click", () => setAll(false));

  // ------------------------------------------------------------------ copy
  const copyText = async (text) => {
    try {
      await navigator.clipboard.writeText(text);
      return true;
    } catch {
      const area = document.createElement("textarea");
      area.value = text;
      area.style.position = "fixed";
      area.style.opacity = "0";
      document.body.appendChild(area);
      area.select();
      const ok = document.execCommand("copy");
      area.remove();
      return ok;
    }
  };

  const flash = (element, text) => {
    const original = element.textContent;
    element.textContent = text;
    element.classList.add("copied");
    setTimeout(() => {
      element.textContent = original;
      element.classList.remove("copied");
    }, 1200);
  };

  document.addEventListener("click", async (event) => {
    const button = event.target.closest(".copy");
    if (button) {
      const pre = button.parentElement.querySelector("pre");
      if (!pre) return;
      if (await copyText(pre.innerText)) {
        flash(button, "copied");
      }
      return;
    }

    const idButton = event.target.closest(".copy-id");
    if (idButton) {
      if (await copyText(idButton.dataset.copy || "")) {
        flash(idButton, "copied");
      }
    }
  });
})();
