import { useCallback, useEffect, useRef, useState } from "react";
import { chordKeys } from "../chords";
import { ArrowDownIcon, ArrowUpIcon, CloseIcon, FileIcon, FolderIcon, PlusIcon } from "../icons";
import { type Browsing, childOf, downloadServerFile, endBrowsing, parentOf, useBrowsing } from "../serverFiles";
import { sizeText } from "../sizes";
import { Refusal } from "../ui/Refusal";
import { wire } from "../wire";

type Listing = { path: string; parent: string | null; entries: { name: string; path: string; directory: boolean; size: number }[] };

/**
 * The window's own view of a server's disk, for a desk on a server
 * (serverFiles.ts). It chooses a folder where this computer's desk would open
 * the system's folder chooser, and shows a folder where it would open the
 * file manager, with a way to bring any file in it down.
 */
export function ServerFiles() {
	const browsing = useBrowsing();
	if (browsing === null) return null;
	return <Browser key={`${browsing.mode}:${browsing.start}`} browsing={browsing} />;
}

function Browser({ browsing }: { browsing: Browsing }) {
	const [listing, setListing] = useState<Listing | null>(null);
	const [typed, setTyped] = useState(browsing.start);
	const [refusal, setRefusal] = useState<string | null>(null);
	const [naming, setNaming] = useState<string | null>(null);
	const [busy, setBusy] = useState<string | null>(null);
	const panel = useRef<HTMLDivElement>(null);

	const go = useCallback((path: string) => {
		setRefusal(null);
		wire.command("files.browse", { path }).then(
			(next) => {
				setListing(next);
				setTyped(next.path);
			},
			(error: unknown) => setRefusal(error instanceof Error ? error.message : String(error)),
		);
	}, []);

	// A file opens on its folder. A folder that is not there yet (a working
	// directory made on first run) opens at the nearest one that is, and says so.
	useEffect(() => {
		let gone = false;
		void (async () => {
			let path = browsing.start;
			for (let climbs = 0; climbs < 32; climbs++) {
				try {
					const next = await wire.command("files.browse", { path });
					if (gone) return;
					setListing(next);
					setTyped(next.path);
					const found = next.entries.some((entry) => entry.path === browsing.start);
					if (path !== browsing.start && !found) setRefusal(`${browsing.start} is not on the server yet.`);
					return;
				} catch (error) {
					if (gone) return;
					const up = parentOf(path);
					if (up === path) {
						setRefusal(error instanceof Error ? error.message : String(error));
						return;
					}
					path = up;
				}
			}
		})();
		panel.current?.focus();
		return () => {
			gone = true;
		};
	}, [browsing.start]);

	useEffect(() => {
		const onKey = (event: KeyboardEvent) => {
			// A new folder's name puts itself down first.
			if (event.key !== "Escape" || (event.target as Element | null)?.closest("[data-naming]")) return;
			event.preventDefault();
			event.stopPropagation();
			endBrowsing(null);
		};
		window.addEventListener("keydown", onKey, true);
		return () => window.removeEventListener("keydown", onKey, true);
	}, []);

	const makeFolder = async () => {
		if (listing === null || naming === null || naming.trim() === "") return;
		const path = childOf(listing.path, naming.trim());
		try {
			await wire.command("files.mkdir", { path });
			setNaming(null);
			go(path);
		} catch (error) {
			setRefusal(error instanceof Error ? error.message : String(error));
		}
	};

	const bringDown = async (path: string) => {
		setBusy(path);
		setRefusal(null);
		try {
			await downloadServerFile(path);
		} catch (error) {
			setRefusal(error instanceof Error ? error.message : String(error));
		} finally {
			setBusy(null);
		}
	};

	const entries = listing === null ? [] : [...listing.entries].sort((a, b) => Number(b.directory) - Number(a.directory));
	const choosing = browsing.mode === "folder";

	return (
		<div className="server-files-scrim" onMouseDown={(event) => event.target === event.currentTarget && endBrowsing(null)}>
			<div ref={panel} tabIndex={-1} className="server-files" role="dialog" aria-label={browsing.title}>
				<div className="flex items-center gap-2 p-3 pb-2">
					<h2 className="min-w-0 flex-1 truncate pl-1 font-semibold">{browsing.title}</h2>
					<button type="button" className="control btn-icon" title={`Close (${chordKeys("close")})`} aria-label="Close" onClick={() => endBrowsing(null)}>
						<CloseIcon />
					</button>
				</div>
				<div className="flex items-center gap-2 px-3 pb-2">
					<button
						type="button"
						className="control btn btn-icon"
						title="Up one folder"
						aria-label="Up one folder"
						disabled={listing?.parent == null}
						onClick={() => go(listing?.parent ?? parentOf(typed))}
					>
						<ArrowUpIcon />
					</button>
					<input
						className="field min-w-0 flex-1 font-mono text-sm"
						aria-label="Folder on the server"
						spellCheck={false}
						value={typed}
						onChange={(event) => setTyped(event.target.value)}
						onKeyDown={(event) => {
							if (event.key === "Enter") go(typed.trim());
						}}
					/>
				</div>
				<ul className="server-files-list" aria-label="In this folder">
					{listing !== null && entries.length === 0 && <li className="px-3 py-2 text-ink-3">This folder is empty.</li>}
					{entries.map((entry) => (
						<li key={entry.path}>
							{entry.directory ? (
								<button type="button" className="server-files-row" onClick={() => go(entry.path)}>
									<FolderIcon className="shrink-0 text-ink-3" />
									<span className="min-w-0 flex-1 truncate">{entry.name}</span>
								</button>
							) : (
								<div className={`server-files-row${entry.path === (browsing.mode === "show" ? browsing.highlight : undefined) ? " row-lit" : ""}`} aria-disabled={choosing}>
									<FileIcon className="shrink-0 text-ink-3" />
									<span className="min-w-0 flex-1 truncate">{entry.name}</span>
									<span className="text-xs text-ink-3">{sizeText(entry.size)}</span>
									{!choosing && (
										<button
											type="button"
											className="control btn-icon"
											title="Save a copy on this computer"
											aria-label={`Save a copy of ${entry.name} on this computer`}
											disabled={busy !== null}
											onClick={() => void bringDown(entry.path)}
										>
											<ArrowDownIcon />
										</button>
									)}
								</div>
							)}
						</li>
					))}
				</ul>
				{refusal !== null && (
					<div className="px-3 pt-2">
						<Refusal message={refusal} />
					</div>
				)}
				{choosing && (
					<div className="flex items-center gap-2 p-3">
						{naming === null ? (
							<button type="button" className="control btn" disabled={listing === null} onClick={() => setNaming("")}>
								<PlusIcon />
								New folder
							</button>
						) : (
							<input
								className="field min-w-0 flex-1 text-sm"
								data-naming
								aria-label="New folder name"
								placeholder="Folder name"
								autoFocus
								value={naming}
								onChange={(event) => setNaming(event.target.value)}
								onBlur={() => naming.trim() === "" && setNaming(null)}
								onKeyDown={(event) => {
									if (event.key === "Enter") void makeFolder();
									if (event.key === "Escape") {
										event.stopPropagation();
										setNaming(null);
									}
								}}
							/>
						)}
						<span className="flex-1" />
						<button type="button" className="control btn" onClick={() => endBrowsing(null)}>
							Cancel
						</button>
						<button type="button" className="control btn btn-primary" disabled={listing === null} onClick={() => listing && endBrowsing(listing.path)}>
							Choose this folder
						</button>
					</div>
				)}
			</div>
		</div>
	);
}
