import { useState } from "react";

/*
 * A site's own icon, the one place in a turn's work an icon earns its room:
 * the pages a teammate read. It is asked of the site itself, as the page's
 * link card asks for its picture, never of a service that would learn what
 * was read; a site without one shows its initial instead.
 */

export function Favicon({ site, size = 16 }: { site: string; size?: number }) {
	const [failed, setFailed] = useState(false);
	const style = { width: size, height: size };
	if (failed) {
		return (
			<span aria-hidden="true" className="favicon favicon-letter" style={{ ...style, fontSize: Math.round(size * 0.62) }}>
				{site.charAt(0).toUpperCase()}
			</span>
		);
	}
	return <img aria-hidden="true" alt="" className="favicon" style={style} src={`https://${site}/favicon.ico`} loading="lazy" onError={() => setFailed(true)} />;
}

/** A few sites' icons, overlapping like a hand of cards: the pages read so far. */
export function Favicons({ sites, most = 4, size = 16 }: { sites: string[]; most?: number; size?: number }) {
	if (sites.length === 0) return null;
	return (
		<span className="favicons">
			{sites.slice(0, most).map((site) => (
				<Favicon key={site} site={site} size={size} />
			))}
		</span>
	);
}
