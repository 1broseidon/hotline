import { chordKeys } from "../chords";
import { CloseIcon, RevealIcon } from "../icons";
import { appVersion, dataDirectory, revealPath } from "../native";
import { Band } from "../ui/Band";
import { ToadMark } from "../ui/ToadMark";
import { Scroll } from "../ui/Scroll";

/** README.md's first sentence. The pane is not a reader of that file. */
const WHAT = "A local-first room for your team of coding agents.";

/**
 * Who this window is, and where it keeps the room. The version and the
 * data directory are facts the shell injected; a browser tab has neither.
 */
export function About({ onClose }: { onClose(): void }) {
	const version = appVersion();
	const dir = dataDirectory();

	return (
		<div className="pane">
			<Band>
				<h2 className="min-w-0 flex-1 truncate pl-1 text-lg font-semibold">About Toad</h2>
				<button
					type="button"
					className="control btn-icon"
					title={`Close (${chordKeys("close")})`}
					aria-label="Close"
					onClick={onClose}
				>
					<CloseIcon />
				</button>
			</Band>
			<Scroll>
				<div className="pane-column flex flex-col gap-6">
					<section className="flex items-start gap-4">
						<ToadMark className="mt-1 shrink-0 text-ink-3" width={44} />
						<div>
							<h3 className="text-xl font-semibold">Toad</h3>
							<p className="mt-1 text-ink-2">{WHAT}</p>
						</div>
					</section>
					{(version !== "" || dir !== "") && (
						<section>
							<h3 className="group-title">This desk</h3>
							<div className="grouped">
								{version !== "" && (
									<div className="group-row">
										<span className="group-row-text">
											<span className="group-row-title">Version</span>
										</span>
										<span className="selectable font-mono text-sm text-ink-2">{version}</span>
									</div>
								)}
								{dir !== "" && (
									<div className="group-row">
										<span className="group-row-text min-w-0">
											<span className="group-row-title">Data directory</span>
											<span className="group-row-detail selectable font-mono" style={{ whiteSpace: "normal" }}>
												{dir}
											</span>
										</span>
										<button
											type="button"
											className="control btn-quiet btn-sm gap-1"
											title="Reveal in the file manager"
											onClick={() => void revealPath(dir)}
										>
											<RevealIcon />
											Reveal
										</button>
									</div>
								)}
							</div>
						</section>
					)}
				</div>
			</Scroll>
		</div>
	);
}
