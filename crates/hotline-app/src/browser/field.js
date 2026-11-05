// What a field is, as the core decides which part of a login it may take
// (`Field::takes` in hotline-core's browser.rs, docs/security.md "Logins in
// the desk browser"). walk.js goes in front, for `clean` and `labelOf`. The
// desk reads these before it types a secret, and act.js reads them again as
// it types, and types nothing if they changed. The keys are `Field`'s.
const fieldFacts = (el) => {
  const input = el instanceof HTMLInputElement;
  const area = el instanceof HTMLTextAreaElement;
  const tag = input
    ? "input"
    : area
      ? "textarea"
      : el instanceof HTMLSelectElement
        ? "select"
        : el.isContentEditable
          ? "editable"
          : el.tagName.toLowerCase();
  const attr = (name) => clean(el.getAttribute(name)).toLowerCase();
  const form = input || area ? el.form : null;
  return {
    tag,
    type: input ? el.type.toLowerCase() : "",
    autocomplete: attr("autocomplete"),
    inputmode: attr("inputmode"),
    role: attr("role"),
    words: clean([el.getAttribute("name"), el.id, labelOf(el), el.getAttribute("placeholder"), el.getAttribute("aria-label")].join(" "))
      .toLowerCase()
      .slice(0, 300),
    inSearch: !!el.closest('[role="search"], search'),
    withPassword: !!form?.querySelector('input[type="password"]'),
  };
};
