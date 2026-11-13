import { useState } from "react";
import { EyeIcon } from "../../icons";
import { openLink } from "../../native";
import type { Source } from "../../turnFacts";
import { Favicon } from "../Favicon";

/*
 * The web pages a turn read, then those its searches only found, by name,
 * each a press away from the page in the person's browser: the top of a
 * turn's path, in the pane and where it opens in place in a thread.
 */

/** How many sources show before the rest are asked for: a turn most often reads a few. */
const SOURCES_SHOWN = 4;

/** The pages the turn read, then those it only found: the first few as cards, the rest a press away. */
export function Sources({ sources }: { sources: Source[] }) {
	const [all, setAll] = useState(false);
	const more = sources.length - SOURCES_SHOWN;
	return (
		<section aria-label="Sources">
			<h3 className="work-heading">Sources · {sources.length}</h3>
			<div className="source-grid">
				{(all ? sources : sources.slice(0, SOURCES_SHOWN)).map((source) => (
					<SourceCard key={source.url} source={source} />
				))}
			</div>
			{more > 0 && (
				<button type="button" className="source-more" aria-expanded={all} onClick={() => setAll((was) => !was)}>
					{all ? "Show fewer" : `Show ${more} more`}
				</button>
			)}
		</section>
	);
}

/**
 * A page's title as words: a search result's title can arrive as the page's
 * markdown, a logo's image and link and all, which reads as noise on a card.
 */
export function plainTitle(title: string | undefined): string | undefined {
	if (title === undefined) return undefined;
	const words = title
		// A link's address may be cut off with the title: to its end, then.
		.replace(/!\[[^\]]*\](\([^)]*\)?)?/g, "")
		.replace(/\[([^\]]*)\]\([^)]*\)?/g, "$1")
		.replace(/[[\]]/g, "")
		.replace(/\s+/g, " ")
		.trim();
	return words === "" ? undefined : words;
}

/** "/blog/next-15-4" for a page with no title of its own: where on the site it is. */
export function pathOf(url: string): string {
	try {
		const at = new URL(url);
		const path = decodeURIComponent(at.pathname).replace(/\/$/, "");
		return path === "" ? at.hostname.replace(/^www\./, "") : path;
	} catch {
		return url;
	}
}

/** The green eye on a page the agent opened, as against one a search only found. */
export function Read() {
	return (
		<span className="source-read" title="Read" aria-label="Read">
			<EyeIcon />
		</span>
	);
}

/** A page the turn read or found: its site's icon and name, and its title, a press away from the page. */
function SourceCard({ source }: { source: Source }) {
	const title = plainTitle(source.title);
	const titled = title !== undefined;
	return (
		<a
			href={source.url}
			className="source-card"
			data-found={source.found || undefined}
			onClick={(click) => {
				click.preventDefault();
				void openLink(source.url);
			}}
		>
			<span className="source-site">
				<Favicon site={source.site} size={16} />
				<span className="min-w-0 flex-1 truncate">{source.site}</span>
				{source.found !== true && <Read />}
			</span>
			<span className={titled ? "source-title" : "source-title source-path"}>{titled ? title : pathOf(source.url)}</span>
		</a>
	);
}
