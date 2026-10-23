import { type FocusEvent, Fragment, type KeyboardEvent, useEffect, useId, useLayoutEffect, useRef, useState } from "react";
import type { AccountSection } from "../../generated/contract";
import { CheckIcon, ChevronDownIcon, PlusIcon, SearchIcon } from "../../icons";
import type { AccountChoice } from "./fits";

const SECTIONS: readonly { section: AccountSection; title: string }[] = [
	{ section: "subscription", title: "Subscriptions" },
	{ section: "api_key", title: "API keys · pay per use" },
	{ section: "server", title: "Servers" },
];

const SEARCH_PAST = 8;

type Adds = { subscription?: () => void; apiKey?: () => void };

// Shut, the picker shows no heading, so the button itself says a key bills per use.
function line(choice: AccountChoice): string {
	return choice.payPerUse ? `${choice.detail} · pay per use` : choice.detail;
}

/** The list hangs from its button inside whatever holds it, so a card or a form never grows when it opens. */
export function AccountPicker({
	choices,
	selected,
	label,
	onSelect,
	adds,
}: {
	choices: AccountChoice[];
	selected: string;
	label: string;
	onSelect(id: string): void;
	adds?: Adds;
}) {
	const button = useRef<HTMLButtonElement>(null);
	const [open, setOpen] = useState(false);
	const current = choices.find((one) => one.id === selected);
	return (
		<div style={{ position: "relative" }}>
			<button
				ref={button}
				type="button"
				className="control picker picker-field"
				aria-label={label}
				aria-haspopup="listbox"
				aria-expanded={open}
				onClick={() => setOpen((was) => !was)}
			>
				<span className="picker-label">
					{current?.name ?? selected}
					{current !== undefined && <span className="text-ink-3"> · {line(current)}</span>}
				</span>
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
		</div>
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
	const refocus = useRef(true);
	const ids = useId();
	const [query, setQuery] = useState("");
	const [above, setAbove] = useState(false);
	const needle = query.trim().toLowerCase();
	const groups = SECTIONS.map(({ section, title }) => ({
		title,
		choices: choices.filter((one) => one.section === section && one.name.toLowerCase().includes(needle)),
	})).filter((group) => group.choices.length > 0);
	const shown = groups.flatMap((group) => group.choices);
	const [active, setActive] = useState(() => Math.max(0, shown.findIndex((one) => one.id === selected)));
	const { subscription, apiKey } = adds ?? {};

	// It opens above when the window has more room there, as the window's other menus do.
	useLayoutEffect(() => {
		const menu = root.current;
		if (!menu) return;
		const at = anchor.getBoundingClientRect();
		const below = window.innerHeight - at.bottom;
		setAbove(menu.offsetHeight > below && at.top > below);
	}, [anchor]);

	useEffect(() => {
		(search.current ?? list.current)?.focus({ preventScroll: true });
		return () => {
			if (refocus.current) anchor.focus({ preventScroll: true });
		};
	}, [anchor]);

	useEffect(() => {
		const away = (event: MouseEvent) => {
			const target = event.target as Node | null;
			if (root.current?.contains(target) || anchor.contains(target)) return;
			onClose();
		};
		document.addEventListener("mousedown", away, true);
		return () => document.removeEventListener("mousedown", away, true);
	}, [anchor, onClose]);

	useEffect(() => {
		document.getElementById(`${ids}-${active}`)?.scrollIntoView({ block: "nearest" });
	}, [ids, active]);

	const onKey = (event: KeyboardEvent) => {
		if (event.key === "Escape") {
			// Ours, not the window's: it would close the pane behind the picker too.
			event.preventDefault();
			event.stopPropagation();
			onClose();
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

	// A focus that leaves the picker for somewhere else closes it, and stays where it went.
	const onLeave = (event: FocusEvent) => {
		const to = event.relatedTarget as Node | null;
		if (to === null || root.current?.contains(to) || anchor.contains(to)) return;
		refocus.current = false;
		onClose();
	};
	const keyed = { onKeyDown: onKey, onBlur: onLeave };

	const addRow = (text: string, add: () => void) => (
		<button
			type="button"
			className="menu-item account-picker-add"
			{...keyed}
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
	return (
		<div
			ref={root}
			className="menu account-picker-menu"
			style={{ position: "absolute", left: 0, ...(above ? { bottom: "calc(100% + 4px)" } : { top: "calc(100% + 4px)" }) }}
		>
			{choices.length > SEARCH_PAST && (
				<label className="search-field account-picker-search">
					<SearchIcon className="shrink-0" />
					<input
						ref={search}
						type="search"
						placeholder={`Search ${choices.length} accounts`}
						aria-label={`Search ${choices.length} accounts`}
						value={query}
						spellCheck={false}
						{...keyed}
						onChange={(event) => {
							setQuery(event.target.value);
							setActive(0);
						}}
					/>
				</label>
			)}
			<div
				ref={list}
				role="listbox"
				aria-label={label}
				tabIndex={-1}
				aria-activedescendant={shown[active] === undefined ? undefined : `${ids}-${active}`}
				className="account-picker-list"
				{...keyed}
			>
				{groups.map((group) => (
					<Fragment key={group.title}>
						<div className="menu-heading" role="presentation">
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
										<span className="menu-item-detail">{choice.detail}</span>
									</span>
									<span className="menu-item-check">{choice.id === selected && <CheckIcon />}</span>
								</button>
							);
						})}
					</Fragment>
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
		</div>
	);
}
