import type { Mood } from "./mood";

const SVG = "http://www.w3.org/2000/svg";

/**
 * A drawing as the picture of one mood: the rig's CSS put inside it and the
 * mood marked on its root, so CSS alone moves its parts. A still picture is
 * the mood's pose with nothing moving. `null` when it is not
 * an SVG. The result is only ever shown as an image, which cannot run script,
 * load anything or follow a link, so nothing in the drawing needs cleaning.
 */
export function moodPicture(svg: string, mood: Mood, still: boolean, rig: string, parser: DOMParser = new DOMParser(), serializer: XMLSerializer = new XMLSerializer()): string | null {
	const doc = parser.parseFromString(svg, "image/svg+xml");
	const root = doc.documentElement;
	if (root === null || root.localName !== "svg" || root.namespaceURI !== SVG || doc.getElementsByTagName("parsererror").length > 0) return null;
	root.setAttribute("data-mood", mood);
	if (still) root.setAttribute("data-still", "");
	const style = doc.createElementNS(SVG, "style");
	style.textContent = rig;
	root.insertBefore(style, root.firstChild);
	return serializer.serializeToString(root);
}
