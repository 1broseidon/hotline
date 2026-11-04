import { useEffect, useState } from "react";
import { fetchIcon, knownIcon } from "../linkPreview";
import { useLinkPreviews } from "../room";

/*
 * A site's own icon, the one place in a turn's work an icon earns its room:
 * the pages a teammate read. The desk fetches it once from the site and
 * keeps it, and the window draws the bytes it hands over, so the window
 * never asks the site itself, nor a service that would learn what was read.
 * A site the desk has no icon for shows nothing: no tile, no initial.
 */

/** Each site's icon as the desk keeps it: a `data:` URL, `null` for none, `undefined` while it is asked. */
function useIcons(sites: readonly string[]): readonly (string | null | undefined)[] {
	const on = useLinkPreviews();
	const key = sites.join("\n");
	const [, redraw] = useState(0);
	useEffect(() => {
		if (on !== true || key === "") return;
		let current = true;
		for (const site of key.split("\n")) {
			if (knownIcon(site) !== undefined) continue;
			void fetchIcon(site).then(() => current && redraw((count) => count + 1));
		}
		return () => {
			current = false;
		};
	}, [key, on]);
	return on === false ? sites.map(() => null) : sites.map(knownIcon);
}

function Icon({ source, size }: { source: string; size: number }) {
	const [failed, setFailed] = useState(false);
	if (failed) return null;
	return <img aria-hidden="true" alt="" className="favicon" style={{ width: size, height: size }} src={source} onError={() => setFailed(true)} />;
}

export function Favicon({ site, size = 16 }: { site: string; size?: number }) {
	const [icon] = useIcons([site]);
	return icon ? <Icon source={icon} size={size} /> : null;
}

/** A few sites' icons, overlapping like a hand of cards: the pages read so far, and how many more. */
export function Favicons({ sites, most = 3, size = 16, total }: { sites: string[]; most?: number; size?: number; total?: number }) {
	const shown = sites.slice(0, most);
	const icons = useIcons(shown);
	const drawn = icons.filter((icon): icon is string => typeof icon === "string");
	if (drawn.length === 0) return null;
	// "+N" counts what the icons stand for: the pages, when `total` says how many, so it agrees with the pane's count.
	const more = (total ?? sites.length) - drawn.length;
	return (
		<span className="favicons">
			{drawn.map((icon, index) => (
				// biome-ignore lint/suspicious/noArrayIndexKey: two sites may share one icon; the order is the sites'.
				<Icon key={index} source={icon} size={size} />
			))}
			{more > 0 && <span className="favicons-more">+{more}</span>}
		</span>
	);
}
