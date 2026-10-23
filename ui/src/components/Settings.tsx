import { useEffect, useState } from "react";
import { appVersion, watchUpdates } from "../native";
import { chordKeys } from "../chords";
import { ArrowLeftIcon } from "../icons";
import { useRoomSettings } from "../room";
import { BackKey, Band } from "../ui/Band";
import { Picker } from "../ui/Menu";
import { Refusal } from "../ui/Refusal";
import { Scroll } from "../ui/Scroll";
import { wire } from "../wire";
import { AboutSection } from "./About";
import { SecretsSection } from "./Secrets";
import { ShortcutsSection } from "./Shortcuts";
import { SkillsSection } from "./Skills";
import { UpdatesSection } from "./UpdatesSection";
import { RemoteSection } from "./RemoteSection";
import { onServer } from "../serverFiles";
import { DesksSection } from "./DesksSection";
import { useActiveDesk, useDesks } from "../desks";
import type { RosterEntry } from "../generated/contract";
import { ComputerSection } from "./settings/Computer";
import { GeneralSection } from "./settings/General";
import { ImportSection } from "./settings/Import";
import { ProvidersSection } from "./settings/Providers";
import { ToolsSection } from "./settings/Tools";

export { McpOAuthControls } from "./settings/Tools";

export type SettingsSection =
	| "general"
	| "desks"
	| "providers"
	| "tools"
	| "skills"
	| "computer"
	| "secrets"
	| "remote"
	| "updates"
	| "import"
	| "shortcuts"
	| "about";

const SECTIONS: { id: SettingsSection; title: string }[] = [
	{ id: "general", title: "General" },
	{ id: "desks", title: "Desks" },
	{ id: "providers", title: "Providers" },
	{ id: "tools", title: "Tools" },
	{ id: "skills", title: "Skills" },
	{ id: "computer", title: "Computer" },
	{ id: "secrets", title: "Secrets" },
	{ id: "remote", title: "Remote" },
	{ id: "updates", title: "Updates" },
	{ id: "import", title: "Import" },
	{ id: "shortcuts", title: "Keyboard shortcuts" },
	{ id: "about", title: "About Hotline" },
];

/** About and Shortcuts are routable sections that stand in the rail's footer, not its list. */
const FOOTER_SECTIONS: SettingsSection[] = ["shortcuts", "about"];

/**
 * The rail while settings are open: the sections stand where the team
 * stood, one row each, and the band carries the way back where the team's
 * plus was. Settings is a place you go, not a card over the room, so the
 * room steps aside until you come back.
 */
export function SettingsRail({
	section,
	onSection,
	onBack,
	onDesk,
	width,
}: {
	section: SettingsSection;
	onSection(section: SettingsSection): void;
	onBack(): void;
	/** Show another desk's settings: the window moves there, as it does for a teammate (BRO-178). */
	onDesk?: ((deskId: string) => void) | undefined;
	/** The rail's dragged width: settings' menu stands where the rail does. */
	width?: number | undefined;
}) {
	const desks = useDesks();
	const here = useActiveDesk();
	return (
		<nav aria-label="Settings" className="rail flex flex-col" style={width !== undefined ? { width } : undefined}>
			<Band>
				<button
					type="button"
					className="control btn-icon -ml-1"
					title={`Back (${chordKeys("close")})`}
					aria-label="Back to the team"
					onClick={onBack}
				>
					<ArrowLeftIcon />
				</button>
				<h1 className="eyebrow min-w-0 flex-1 truncate pl-1">Settings</h1>
			</Band>
			{/* Settings are a desk's: the desk of the teammate open, or another one picked here. */}
			{desks.length > 1 && here !== null && onDesk !== undefined && (
				<div className="shrink-0 px-2 pt-1">
					<Picker
						field
						value={here.id}
						choices={desks.map((desk) => ({ id: desk.id, name: desk.name, group: desk.kind === "local" ? "On this computer" : "Remote" }))}
						placeholder="Desk"
						label="Whose settings"
						onChange={(deskId) => {
							if (deskId !== here.id) onDesk(deskId);
						}}
					/>
				</div>
			)}
			<div className="min-h-0 flex-1 overflow-y-auto px-2 pb-2 pt-1">
				{SECTIONS.filter((one) => !FOOTER_SECTIONS.includes(one.id)).map((one) => (
					<button
						key={one.id}
						type="button"
						className="rail-row"
						aria-current={section === one.id ? "true" : undefined}
						onClick={() => onSection(one.id)}
					>
						<span className="block min-w-0 flex-1 truncate font-medium text-ink">{one.title}</span>
					</button>
				))}
			</div>
			<RailFooter section={section} onSection={onSection} />
		</nav>
	);
}

/**
 * The quiet foot of the settings rail, where the team rail's dock stands: the
 * version opens About, and Shortcuts opens the chord list. It does not scroll
 * with the sections above it.
 */
function RailFooter({ section, onSection }: { section: SettingsSection; onSection(section: SettingsSection): void }) {
	const [updateReady, setUpdateReady] = useState(false);
	useEffect(() => watchUpdates((status) => setUpdateReady(status.available !== null), () => {}), []);

	const version = appVersion();
	const shortcutsChord = chordKeys("shortcuts");
	const aboutTip = updateReady ? "Update ready" : "About Hotline";
	return (
		<div className="rail-foot">
			<button
				type="button"
				className="rail-foot-item"
				title={aboutTip}
				aria-label={version === "" ? "About Hotline" : `About Hotline, version ${version}`}
				aria-current={section === "about" ? "true" : undefined}
				onClick={() => onSection("about")}
			>
				{version === "" ? "About" : `Hotline ${version}`}
				{updateReady && <span className="rail-foot-dot" aria-hidden="true" />}
			</button>
			<button
				type="button"
				className="rail-foot-item"
				title={shortcutsChord === "" ? "Keyboard shortcuts" : `Keyboard shortcuts (${shortcutsChord})`}
				aria-label="Keyboard shortcuts"
				aria-current={section === "shortcuts" ? "true" : undefined}
				onClick={() => onSection("shortcuts")}
			>
				Shortcuts
			</button>
		</div>
	);
}

/**
 * The room's settings, as a pane in the conversation's place: one section
 * at a time, chosen in the rail, each a column of grouped rows. What a
 * teammate is, is not here; that is the teammate's own pane.
 */
export function Settings({
	section,
	roster,
	account,
	onSection,
	onBack,
	onAddDesk,
}: {
	section: SettingsSection;
	roster: RosterEntry[];
	account?: string | undefined;
	onSection?: (section: SettingsSection) => void;
	onBack?: () => void;
	onAddDesk?: () => void;
}) {
	const settings = useRoomSettings();
	const [refusal, setRefusal] = useState<string | null>(null);

	const patch = (patch: Record<string, unknown>) => {
		setRefusal(null);
		void wire.command("settings.update", { patch }).catch((error: Error) => setRefusal(error.message));
	};

	// Providers, Tools and Skills have a page under them, so their bands are
	// their own; Secrets keeps its own list, so it draws itself the same way.
	if (section === "providers") {
		// Keyed so that opening on another account remounts the section instead of keeping the page it was left on.
		return <ProvidersSection key={account ?? ""} enabledModels={settings.enabledModels} roster={roster} opening={account} onBack={onBack} />;
	}
	if (section === "tools") return <ToolsSection servers={settings.mcpServers} onBack={onBack} />;
	if (section === "skills") return <SkillsSection onBack={onBack} />;
	if (section === "secrets") return <SecretsSection onBack={onBack} />;

	return (
		<div className="pane">
			<Band>
				{onBack !== undefined && <BackKey onBack={onBack} />}
				<h2 className="min-w-0 flex-1 truncate pl-1 text-lg font-semibold">
					{SECTIONS.find((one) => one.id === section)?.title}
				</h2>
			</Band>
			<Scroll>
				<div className="pane-column flex flex-col gap-6">
					{section === "general" && (
						<GeneralSection
							personName={settings.personName}
							onPersonName={(name) => patch({ personName: name })}
							idleHours={settings.chapterIdleHours}
							defaultBackendId={settings.defaultBackendId}
							defaultModelId={settings.defaultModelId}
							onIdleHours={(hours) => patch({ chapterIdleHours: hours })}
							onBackend={(id) => patch({ defaultBackendId: id })}
							onDefaultModel={(id) => patch({ defaultModelId: id })}
							onProviders={onSection === undefined ? undefined : () => onSection("providers")}
						/>
					)}
					{section === "computer" && (
						<ComputerSection
							runtime={settings.computerRuntime}
							image={settings.computerImage}
							onRuntime={(id) => patch({ computerRuntime: id })}
							onImage={(image) => patch({ computerImage: image })}
						/>
					)}
					{section === "updates" && <UpdatesSection />}
					{section === "remote" && (onServer() ? <ServerRemoteNote /> : <RemoteSection />)}
					{section === "desks" && <DesksSection onAddDesk={onAddDesk ?? (() => {})} />}
					{section === "import" && <ImportSection onRefuse={setRefusal} />}
					{section === "shortcuts" && <ShortcutsSection />}
					{section === "about" && <AboutSection />}
					{refusal !== null && <Refusal message={refusal} />}
				</div>
			</Scroll>
		</div>
	);
}


/**
 * A desk on a server keeps its listener and its devices to itself: a remote
 * owner cannot pair more devices or revoke them, so the section says where
 * that is done instead.
 */
function ServerRemoteNote() {
	return (
		<section>
			<h3 className="group-title">Phones and other computers</h3>
			<p className="group-hint">
				This desk runs on a server, and it pairs devices there. On the server, <code>hotline pair</code> shows a QR code for
				the phone, <code>hotline devices</code> lists what is paired, and <code>hotline revoke</code> removes one.
			</p>
		</section>
	);
}
