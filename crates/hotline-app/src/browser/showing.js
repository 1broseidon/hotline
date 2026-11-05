// The body of an async function: what a picture of the page could show, as
// words, for the core to look for a teammate's secret in before and after
// it takes one. The page's rendered text, the text of open shadow roots and
// of frames of its own origin, and the value of every field but a hidden
// one and a password field the page keeps masked. The secrets themselves
// never come here: the core looks for them in what this answers.
// Returns JSON: { text, fields }.
const texts = [document.body?.innerText || ""];
const fields = [];
const look = (root) => {
  for (const el of root.querySelectorAll("*")) {
    if (el.shadowRoot) {
      texts.push(el.shadowRoot.textContent || "");
      look(el.shadowRoot);
    }
    if (el.tagName === "IFRAME") {
      try {
        const doc = el.contentDocument;
        if (doc) {
          texts.push(doc.body?.innerText || "");
          look(doc);
        }
      } catch {
        // Another origin's frame: the desk types nothing there.
      }
    }
    if ((el.tagName === "INPUT" || el.tagName === "TEXTAREA") && el.value) {
      const type = el.tagName === "INPUT" ? (el.type || "").toLowerCase() : "";
      if (type === "hidden") continue;
      const masked = type === "password" && getComputedStyle(el).getPropertyValue("-webkit-text-security") !== "none";
      if (!masked) fields.push(el.value);
    }
  }
};
look(document);
return JSON.stringify({ text: texts.join("\n"), fields });
