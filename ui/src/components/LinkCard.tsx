import { useEffect, useState } from "react";
import type { LinkPreview } from "../generated/contract";
import { fetchPreview, knownPreview } from "../linkPreview";
import { openLink } from "../native";

/** How long a link has to stay the same before it is read, so a reply still arriving does not read every half of it. */
const SETTLE_MS = 600;

/**
 * A link's card under its message, as on the phone: the page's picture, its
 * title and where it lives. It appears only once there is something to show,
 * and a page with nothing to say about itself gets no card, unless the
 * message was only the link, which then draws plainly rather than not at all.
 */
export function LinkCard({ url, always = false, mine = false }: { url: string; always?: boolean; mine?: boolean }) {
	const [preview, setPreview] = useState<LinkPreview | null | undefined>(() => knownPreview(url));
	const [pictureFailed, setPictureFailed] = useState(false);
	useEffect(() => {
		setPreview(knownPreview(url));
		setPictureFailed(false);
		if (knownPreview(url) !== undefined) return;
		let current = true;
		const wait = setTimeout(() => void fetchPreview(url).then((found) => current && setPreview(found)), SETTLE_MS);
		return () => {
			current = false;
			clearTimeout(wait);
		};
	}, [url]);
	if (!preview && !always) return null;
	const shown = preview ?? plain(url);
	const picture = shown.image !== undefined && !pictureFailed;
	return (
		<a
			href={shown.url}
			className={`link-card ${mine ? "link-card-mine" : ""}`}
			title={shown.url}
			onClick={(click) => {
				click.preventDefault();
				void openLink(shown.url);
			}}
		>
			{picture && <img src={shown.image} alt="" loading="lazy" className="link-card-picture" onError={() => setPictureFailed(true)} />}
			<span className="link-card-words">
				<span className="link-card-title">{shown.title}</span>
				<span className="link-card-site">{shown.site}</span>
			</span>
		</a>
	);
}

function plain(url: string): LinkPreview {
	const at = new URL(url);
	const host = at.hostname.replace(/^www\./, "");
	return { url, title: `${host}${at.pathname === "/" ? "" : at.pathname}`, site: host };
}
