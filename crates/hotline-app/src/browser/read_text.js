// The body of an async function: walk.js goes in front of it. The page as
// lines, with refs. Returns JSON: { title, url, text }.
return JSON.stringify({ title: document.title, url: location.href, text: lines.join("\n") });
