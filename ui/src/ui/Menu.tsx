import {
	useEffect,
	useLayoutEffect,
	useRef,
	useState,
	type CSSProperties,
	type KeyboardEvent,
	type ReactNode,
} from "react";
import { createPortal } from "react-dom";
import { CheckIcon, ChevronDownIcon } from "../icons";

/**
 * The window's one popover: a pull-down's list, the overflow menu, a form's
 * choice. It hangs under its anchor, flips above when the window is short,
 * and is driven by the keyboard the way a native menu is — arrows, Home and
 * End, Enter, Escape, and typing a letter to jump. Click-away closes it.
 */

export type MenuEntry =
	| {
			kind: "item";
			id: string;
			text: string;
			detail?: string;
			shortcut?: string;
			checked?: boolean;
			disabled?: boolean;
			danger?: boolean;
			onSelect(): void;
	  }
	| { kind: "heading"; text: string }
	| { kind: "rule" };

export function Menu({
	anchor,
	entries,
	align = "start",
	highlightChecked = false,
	onClose,
}: {
	anchor: HTMLElement;
	entries: MenuEntry[];
	align?: "start" | "end";
	/** Open on the checked item, the way a pull-down does; a menu of actions opens on nothing. */
	highlightChecked?: boolean;
	onClose(): void;
}) {
	const root = useRef<HTMLDivElement>(null);
	const [style, setStyle] = useState<CSSProperties>({ visibility: "hidden" });
	const items = entries.filter((one): one is Extract<MenuEntry, { kind: "item" }> => one.kind === "item");
	const [active, setActive] = useState(() =>
		highlightChecked ? items.findIndex((one) => one.checked && !one.disabled) : -1,
	);

	useLayoutEffect(() => {
		const el = root.current;
		if (!el) return;
		const at = anchor.getBoundingClientRect();
		const height = el.offsetHeight;
		const gap = 4;
		const roomBelow = window.innerHeight - at.bottom - gap - 8;
		const above = height > roomBelow && at.top - gap - 8 > roomBelow;
		const top = above ? Math.max(8, at.top - gap - height) : at.bottom + gap;
		const maxHeight = above ? at.top - gap - 8 : roomBelow;
		// The height is capped before the width is read. A list taller than
		// its room gets a scrollbar, and WebKit sizes a shrink-to-fit box to
		// its content first and adds the bar's width on the next layout — so
		// a width read before the cap is one the menu will not keep. Read it
		// with the bar in place, then pin it, so no later layout can move it.
		el.style.maxHeight = `${maxHeight}px`;
		const width = el.offsetWidth;
		let left = align === "end" ? at.right - width : at.left;
		left = Math.min(Math.max(8, left), window.innerWidth - width - 8);
		setStyle({ top, left, width, maxHeight });
	}, [anchor, align]);

	// Focus once the menu is placed: it opens hidden until it is measured,
	// and a hidden element cannot take focus. Without it, Escape reaches the
	// window instead of the menu and closes whatever pane it opened in.
	const placed = style.visibility !== "hidden";
	useEffect(() => {
		if (placed) root.current?.focus({ preventScroll: true });
	}, [placed]);

	useEffect(() => {
		const away = (event: MouseEvent) => {
			const target = event.target as Node | null;
			if (root.current?.contains(target) || anchor.contains(target)) return;
			onClose();
		};
		const resize = () => onClose();
		document.addEventListener("mousedown", away, true);
		window.addEventListener("resize", resize);
		window.addEventListener("blur", resize);
		return () => {
			document.removeEventListener("mousedown", away, true);
			window.removeEventListener("resize", resize);
			window.removeEventListener("blur", resize);
			anchor.focus({ preventScroll: true });
		};
	}, [anchor, onClose]);

	useEffect(() => {
		root.current
			?.querySelector<HTMLElement>(`[data-index="${active}"]`)
			?.scrollIntoView({ block: "nearest" });
	}, [active]);

	const move = (from: number, step: 1 | -1) => {
		if (items.length === 0) return;
		let next = from;
		for (let tries = 0; tries < items.length; tries++) {
			next = (next + step + items.length) % items.length;
			if (!items[next]?.disabled) {
				setActive(next);
				return;
			}
		}
	};

	const choose = (index: number) => {
		const item = items[index];
		if (!item || item.disabled) return;
		onClose();
		item.onSelect();
	};

	const onKey = (event: KeyboardEvent) => {
		if (event.key === "Escape") {
			event.preventDefault();
			event.stopPropagation();
			onClose();
		} else if (event.key === "ArrowDown") {
			event.preventDefault();
			move(active, 1);
		} else if (event.key === "ArrowUp") {
			event.preventDefault();
			move(active, -1);
		} else if (event.key === "Home") {
			event.preventDefault();
			move(-1, 1);
		} else if (event.key === "End") {
			event.preventDefault();
			move(0, -1);
		} else if (event.key === "Enter" || event.key === " ") {
			event.preventDefault();
			choose(active);
		} else if (event.key === "Tab") {
			event.preventDefault();
			onClose();
		} else if (event.key.length === 1 && !event.ctrlKey && !event.metaKey && !event.altKey) {
			const letter = event.key.toLowerCase();
			const from = active;
			for (let step = 1; step <= items.length; step++) {
				const index = (from + step) % items.length;
				const item = items[index];
				if (item && !item.disabled && item.text.toLowerCase().startsWith(letter)) {
					setActive(index);
					return;
				}
			}
		}
	};

	let index = -1;
	return createPortal(
		<div
			ref={root}
			role="menu"
			tabIndex={-1}
			className="menu"
			style={style}
			onKeyDown={onKey}
			onMouseLeave={() => setActive(-1)}
		>
			{entries.map((entry, at) => {
				if (entry.kind === "rule") return <div key={`rule-${at}`} className="menu-rule" role="separator" />;
				if (entry.kind === "heading") {
					return (
						<div key={`heading-${at}`} className="menu-heading" role="presentation">
							{entry.text}
						</div>
					);
				}
				index++;
				const here = index;
				return (
					<button
						key={entry.id}
						type="button"
						role="menuitem"
						data-index={here}
						data-active={active === here ? "true" : undefined}
						aria-disabled={entry.disabled ? "true" : undefined}
						tabIndex={-1}
						className="menu-item"
						style={entry.danger && active !== here ? { color: "var(--danger)" } : undefined}
						onMouseEnter={() => !entry.disabled && setActive(here)}
						onClick={() => choose(here)}
					>
						<span className="menu-item-check">{entry.checked && <CheckIcon />}</span>
						<span className="menu-item-text">
							{entry.text}
							{entry.detail !== undefined && <span className="menu-item-detail">{entry.detail}</span>}
						</span>
						{entry.shortcut !== undefined && <span className="menu-shortcut">{entry.shortcut}</span>}
					</button>
				);
			})}
		</div>,
		document.body,
	);
}

export type Choice = { id: string; name: string; detail?: string; group?: string };

/**
 * A pull-down: a button naming the current choice, and the menu of the
 * others. `field` frames it like a text field for forms; otherwise it sits
 * flat in a band. A value the list has not named yet is still shown, so
 * the picker never claims the teammate is on something it is not.
 */
export function Picker({
	value,
	choices,
	placeholder,
	label,
	field = false,
	disabled = false,
	onChange,
}: {
	value: string;
	choices: Choice[];
	placeholder: string;
	label: string;
	field?: boolean;
	disabled?: boolean;
	onChange(id: string): void;
}) {
	const button = useRef<HTMLButtonElement>(null);
	const [open, setOpen] = useState(false);
	const current = choices.find((one) => one.id === value);
	const text = current?.name ?? (value !== "" ? value : placeholder);

	const entries: MenuEntry[] = [];
	let group: string | undefined;
	for (const choice of choices) {
		if (choice.group !== undefined && choice.group !== group) {
			if (entries.length > 0) entries.push({ kind: "rule" });
			entries.push({ kind: "heading", text: choice.group });
			group = choice.group;
		}
		entries.push({
			kind: "item",
			id: choice.id,
			text: choice.name,
			...(choice.detail !== undefined ? { detail: choice.detail } : {}),
			checked: choice.id === value,
			onSelect: () => onChange(choice.id),
		});
	}

	return (
		<>
			<button
				ref={button}
				type="button"
				className={`control picker ${field ? "picker-field" : ""}`}
				aria-label={label}
				aria-haspopup="menu"
				aria-expanded={open}
				disabled={disabled}
				title={current?.name ?? text}
				onClick={() => setOpen((was) => !was)}
			>
				<span className="picker-label" style={current === undefined && value === "" ? { color: "var(--ink-3)" } : undefined}>
					{text}
				</span>
				<ChevronDownIcon className="picker-chevron" />
			</button>
			{open && button.current && (
				<Menu anchor={button.current} entries={entries} highlightChecked onClose={() => setOpen(false)} />
			)}
		</>
	);
}

/** A button that opens a menu of actions. */
export function MenuButton({
	entries,
	align = "end",
	className,
	label,
	title,
	children,
}: {
	entries: MenuEntry[];
	align?: "start" | "end";
	className: string;
	label: string;
	title?: string;
	children: ReactNode;
}) {
	const button = useRef<HTMLButtonElement>(null);
	const [open, setOpen] = useState(false);
	return (
		<>
			<button
				ref={button}
				type="button"
				className={className}
				aria-label={label}
				title={title ?? label}
				aria-haspopup="menu"
				aria-expanded={open}
				onClick={() => setOpen((was) => !was)}
			>
				{children}
			</button>
			{open && button.current && (
				<Menu anchor={button.current} entries={entries} align={align} onClose={() => setOpen(false)} />
			)}
		</>
	);
}

/** Arrows walk a `role=tablist`; Tab already lands on each tab. */
export function onTablistKey(event: KeyboardEvent<HTMLElement>): void {
	if (event.key !== "ArrowRight" && event.key !== "ArrowLeft") return;
	const tabs = [...event.currentTarget.querySelectorAll<HTMLElement>("[role=tab]")];
	const from = tabs.indexOf(event.target as HTMLElement);
	if (from < 0) return;
	event.preventDefault();
	const next = tabs[(from + (event.key === "ArrowRight" ? 1 : -1) + tabs.length) % tabs.length];
	next?.focus();
	next?.click();
}
