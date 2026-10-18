import { useState, type ReactNode } from "react";
import { ChevronDownIcon, ChevronRightIcon, InfoIcon } from "../../icons";

/** The words for the access switches, shared with the new-teammate form so both say the same thing. */
export const MACHINE_ABOUT =
	"On gives built-in tools this account's full access and lets it delegate without asking. Off isolates supported built-in tools from unrelated host files and other workspaces, except installed runtimes. Network and granted MCP access remain available.";
export const BACKGROUND_ABOUT =
	"It may set its own schedules and wake itself, including after you stop a session. Off pauses the ones it made; jobs you add here run either way.";
export const COMPUTER_ABOUT =
	"A desktop of its own in a container, driven through its capture, input, browser and shell tools. Off keeps its work on this machine.";

/**
 * A grant that is on or off. A grant with a risk worth a sentence gets an
 * info key after its title; the sentence opens as a line under the row,
 * so the row itself stays one line.
 */
export function SwitchRow({
	title,
	value,
	about,
	checked,
	disabled,
	onChange,
}: {
	title: string;
	value?: string;
	about?: string;
	checked: boolean;
	disabled: boolean;
	onChange(checked: boolean): void;
}) {
	const [told, setTold] = useState(false);
	return (
		<>
			<label className="group-row group-row-choice">
				<RowText title={title} {...(value !== undefined ? { value } : {})} />
				{about !== undefined && (
					<span className="-my-1 -ml-2 flex">
						<InfoKey about={about} label={`About ${title}`} open={told} onToggle={() => setTold((was) => !was)} />
					</span>
				)}
				<input
					type="checkbox"
					className="switch"
					checked={checked}
					disabled={disabled}
					onChange={(event) => onChange(event.target.checked)}
				/>
			</label>
			{told && about !== undefined && (
				<div className={NESTED}>
					<span className="group-row-text">
						<span className="group-row-detail" style={{ whiteSpace: "normal" }}>
							{about}
						</span>
					</span>
				</div>
			)}
		</>
	);
}

/** A row's words: the title, and after a dot the value, if it has one. */
export function RowText({ title, value }: { title: string; value?: string }) {
	return (
		<span className="group-row-text">
			<span className="group-row-title">
				{title}
				{value !== undefined && <span className="text-ink-3"> · {value}</span>}
			</span>
		</span>
	);
}

/** The small key that shows a row's one sentence: on hover as a tooltip, on press as a line under the row. */
export function InfoKey({ about, label, open, onToggle }: { about: string; label: string; open: boolean; onToggle(): void }) {
	return (
		<button
			type="button"
			className="control btn-icon btn-quiet h-6 w-6 text-ink-3"
			title={about}
			aria-label={label}
			aria-expanded={open}
			onClick={onToggle}
		>
			<InfoIcon />
		</button>
	);
}

/** A row with more under it, opened in place. */
export function FoldRow({
	title,
	value,
	open,
	onToggle,
	mark,
}: {
	title: string;
	value?: string;
	open: boolean;
	onToggle(): void;
	mark?: ReactNode;
}) {
	return (
		<button type="button" className="group-row group-row-choice w-full text-left" aria-expanded={open} onClick={onToggle}>
			<RowText title={title} {...(value !== undefined ? { value } : {})} />
			{mark}
			{open ? <ChevronDownIcon className="shrink-0 text-ink-3" /> : <ChevronRightIcon className="shrink-0 text-ink-3" />}
		</button>
	);
}

/** What a fold opens is set in from its row, so the list reads as a tree. */
export const NESTED = "group-row pl-7";
