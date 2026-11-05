import { useEffect, useState } from "react";
import type { LinkPreview } from "../generated/contract";
import { drawable, fetchPreview, knownPreview } from "../linkPreview";
import { Favicon } from "./Favicon";
import { openLink } from "../native";
import { useLinkPreviews } from "../room";

/** How long a link has to stay the same before it is read, so a reply still arriving does not read every half of it. */
const SETTLE_MS = 600;

/**
 * A link's card under its message: where the page lives, its title, two
 * lines of what it says about itself, and its picture only when it has a
 * real one, which the desk fetched and handed over as bytes. No picture is
 * no tile. It appears only once there is something to show, and a page with
 * nothing to say about itself gets no card, unless the message was only the
 * link, which then draws plainly rather than not at all. With link previews
 * off there is no card at all, and nothing is asked.
 */
export function LinkCard({ url, always = false, mine = false }: { url: string; always?: boolean; mine?: boolean }) {
	const on = useLinkPreviews();
	const [preview, setPreview] = useState<LinkPreview | null | undefined>(() => knownPreview(url));
	const [pictureFailed, setPictureFailed] = useState(false);
	useEffect(() => {
		setPreview(knownPreview(url));
		setPictureFailed(false);
		if (on !== true || knownPreview(url) !== undefined) return;
		let current = true;
		const wait = setTimeout(() => void fetchPreview(url).then((found) => current && setPreview(found)), SETTLE_MS);
		return () => {
			current = false;
			clearTimeout(wait);
		};
	}, [url, on]);
	if (on === false || (!preview && !always)) return null;
	const shown = preview ?? plain(url);
	const picture = drawable(shown.image) && !pictureFailed ? shown.image : null;
	return (
		<a
			href={shown.url}
			className={`link-card${picture ? " link-card-pictured" : ""}${mine ? " link-card-mine" : ""}`}
			title={shown.url}
			onClick={(click) => {
				click.preventDefault();
				void openLink(shown.url);
			}}
		>
			<span className="link-card-words">
				<span className="link-card-site">
					<Favicon site={new URL(shown.url).hostname.replace(/^www\./, "")} size={14} />
					{shown.site}
				</span>
				<span className="link-card-title">{shown.title}</span>
				{shown.description !== undefined && <span className="link-card-description">{shown.description}</span>}
			</span>
			{picture && <img src={picture} alt="" className="link-card-picture" onError={() => setPictureFailed(true)} />}
		</a>
	);
}

function plain(url: string): LinkPreview {
	const at = new URL(url);
	const host = at.hostname.replace(/^www\./, "");
	return { url, title: `${host}${at.pathname === "/" ? "" : at.pathname}`, site: host };
}
