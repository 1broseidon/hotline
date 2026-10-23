import { type KeyboardEvent, useEffect, useId, useRef, useState } from "react";
import { createPortal } from "react-dom";
import type { AccountSection } from "../../generated/contract";
import { CheckIcon, ChevronDownIcon, PlusIcon, SearchIcon } from "../../icons";
import { usePlacement } from "../../ui/Menu";
import { type AccountChoice, detailLine, rowDetail } from "./fits";

const SECTIONS: readonly { section: AccountSection; title: string }[] = [
	{ section: "subscription", title: "Subscriptions" },
	{ section: "api_key", title: "API keys · pay per use" },
	{ section: "server", title: "Servers" },
];

const SEARCH_PAST = 8;

type Adds = { subscription?: (() => void) | undefined; apiKey?: (() => void) | undefined };

/** The list floats on the window as its menus do, so no card or row that holds the picker grows or clips it. */
export function AccountPicker({
	choices,
	selected,
	label,
	onSelect,
	adds,
	disabled = false,
}: {
	choices: AccountChoice[];
	selected: string;
	label: string;
	onSelect(id: string): void;
	adds?: Adds;
	disabled?: boolean;
}) {
	const button = useRef<HTMLButtonElement>(null);
	const [open, setOpen] = useState(false);
	const current = choices.find((one) => one.id === selected);
	return (
		<>
			<button
				ref={button}
				type="button"
				className="control picker picker-field"
				aria-label={label}
				aria-haspopup="listbox"
				aria-expanded={open}
				disabled={disabled}
				onClick={() => setOpen((was) => !was)}
			>
				{current === undefined ? (
					<span className="picker-label text-ink-3">Choose an account</span>
				) : (
					<span className="picker-label">
						{current.name}
						<span className="text-ink-3"> · {detailLine(current)}</span>
					</span>
				)}
				<ChevronDownIcon className="picker-chevron" />
			</button>
			{open && button.current && (
				<AccountMenu
					choices={choices}
					selected={selected}
					label={label}
					adds={adds}
					anchor={button.current}
					onClose={() => setOpen(false)}
					onPick={(id) => {
						setOpen(false);
						onSelect(id);
					}}
				/>
			)}
		</>
	);
}

function AccountMenu({
	choices,
	selected,
	label,
	adds,
	anchor,
	onClose,
	onPick,
}: {
	choices: AccountChoice[];
	selected: string;
	label: string;
	adds: Adds | undefined;
	anchor: HTMLButtonElement;
	onClose(): void;
	onPick(id: string): void;
}) {
	const root = useRef<HTMLDivElement>(null);
	const search = useRef<HTMLInputElement>(null);
	const list = useRef<HTMLDivElement>(null);
	const ids = useId();
	const { style, placed } = usePlacement(root, anchor, "start", true);
	const [query, setQuery] = useState("");
	const needle = query.trim().toLowerCase();
	const groups = SECTIONS.map(({ section, title }) => ({
		section,
		title,
		choices: choices.filter((one) => one.section === section && one.name.toLowerCase().includes(needle)),
	})).filter((group) => group.choices.length > 0);
	const shown = groups.flatMap((group) => group.choices);
	const [active, setActive] = useState(() => Math.max(0, shown.findIndex((one) => one.id === selected)));
	const activeId = shown[active] === undefined ? undefined : `${ids}-${active}`;
	const searchable = choices.length > SEARCH_PAST;
	const { subscription, apiKey } = adds ?? {};

	useEffect(() => {
		if (placed) (search.current ?? list.current)?.focus({ preventScroll: true });
	}, [placed]);

	useEffect(() => () => anchor.focus({ preventScroll: true }), [anchor]);

	useEffect(() => {
		const away = (event: MouseEvent) => {
			const target = event.target as Node | null;
			if (root.current?.contains(target) || anchor.contains(target)) return;
			onClose();
		};
		const shut = () => onClose();
		document.addEventListener("mousedown", away, true);
		window.addEventListener("resize", shut);
		window.addEventListener("blur", shut);
		return () => {
			document.removeEventListener("mousedown", away, true);
			window.removeEventListener("resize", shut);
			window.removeEventListener("blur", shut);
		};
	}, [anchor, onClose]);

	// Only the menu's own list can scroll here: the menu sits on the body, so no pane is its ancestor.
	useEffect(() => {
		if (placed && activeId !== undefined) document.getElementById(activeId)?.scrollIntoView({ block: "nearest" });
	}, [placed, activeId]);

	const onKey = (event: KeyboardEvent) => {
		if (event.key === "Escape") {
			// Ours, not the window's: it would close the pane behind the picker too.
			event.preventDefault();
			event.stopPropagation();
			onClose();
			return;
		}
		if (event.key === "Tab") {
			// The menu sits at the end of the body, so a Tab that leaves it would land nowhere near the picker.
			const stops = [search.current ?? list.current, ...(root.current?.querySelectorAll<HTMLElement>(".account-picker-add") ?? [])];
			const next = stops[stops.indexOf(event.target as HTMLElement) + (event.shiftKey ? -1 : 1)];
			event.preventDefault();
			if (next) next.focus({ preventScroll: true });
			else onClose();
			return;
		}
		if (event.target !== search.current && event.target !== list.current) return;
		if (event.key === "ArrowDown" || event.key === "ArrowUp") {
			event.preventDefault();
			if (shown.length === 0) return;
			const step = event.key === "ArrowDown" ? 1 : -1;
			setActive((at) => (at + step + shown.length) % shown.length);
		} else if (event.key === "Enter") {
			const choice = shown[active];
			if (choice === undefined) return;
			event.preventDefault();
			onPick(choice.id);
		}
	};

	const addRow = (text: string, add: () => void) => (
		<button
			type="button"
			className="menu-item account-picker-add"
			onKeyDown={onKey}
			onClick={() => {
				onClose();
				add();
			}}
		>
			<PlusIcon />
			{text}
		</button>
	);

	let index = -1;
	return createPortal(
		<div ref={root} tabIndex={-1} className="menu account-picker-menu" style={style}>
			{searchable && (
				<label className="search-field account-picker-search">
					<SearchIcon className="shrink-0" />
					<input
						ref={search}
						type="search"
						role="combobox"
						aria-expanded={true}
						aria-controls={`${ids}-list`}
						aria-activedescendant={activeId}
						aria-autocomplete="list"
						placeholder={`Search ${choices.length} accounts`}
						aria-label={`Search ${choices.length} accounts`}
						value={query}
						spellCheck={false}
						onKeyDown={onKey}
						onChange={(event) => {
							setQuery(event.target.value);
							setActive(0);
						}}
					/>
				</label>
			)}
			<div
				ref={list}
				id={`${ids}-list`}
				role="listbox"
				aria-label={label}
				tabIndex={-1}
				aria-activedescendant={searchable ? undefined : activeId}
				className="account-picker-list"
				onKeyDown={onKey}
			>
				{groups.map((group) => (
					// biome-ignore lint/a11y/useSemanticElements: a listbox's sections are groups of options, not a form's fieldset.
					<div key={group.section} role="group" aria-labelledby={`${ids}-${group.section}`}>
						<div id={`${ids}-${group.section}`} className="menu-heading">
							{group.title}
						</div>
						{group.choices.map((choice) => {
							index++;
							const here = index;
							return (
								<button
									key={choice.id}
									id={`${ids}-${here}`}
									type="button"
									role="option"
									aria-selected={choice.id === selected}
									data-active={active === here ? "true" : undefined}
									tabIndex={-1}
									className="menu-item"
									onMouseEnter={() => setActive(here)}
									onClick={() => onPick(choice.id)}
								>
									<span className="agent-account-dot" data-state={choice.signedIn ? "signed_in" : undefined} aria-hidden="true" />
									<span className="menu-item-text">
										{choice.name}
										<span className="menu-item-detail">{rowDetail(choice)}</span>
									</span>
									<span className="menu-item-check">{choice.id === selected && <CheckIcon />}</span>
								</button>
							);
						})}
					</div>
				))}
			</div>
			{(subscription !== undefined || apiKey !== undefined) && (
				<>
					<hr className="menu-rule" />
					<div className="account-picker-adds">
						{subscription !== undefined && addRow("Add subscription", subscription)}
						{apiKey !== undefined && addRow("Add API key", apiKey)}
					</div>
				</>
			)}
		</div>,
		document.body,
	);
}
