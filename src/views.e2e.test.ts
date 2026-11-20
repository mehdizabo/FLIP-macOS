import { describe, expect, test } from "bun:test";
import { t } from "./i18n";
import { IMAGE_TOOL_TOKENS } from "./imageTools";
import { defaultPreferences } from "./ipc";
import type {
  AppPreferences,
  ConversionCompletePayload,
  EngineStatus,
  InspectFilesResult,
  NewFileKindInfo,
  NewFileKindsResponse,
  PermissionsStatus,
  QuickActionsStatus,
  UpdateCheckResult,
  WheelStatePayload,
} from "./types";
import { mountBatchView } from "./views/BatchView";
import { mountOnboardingView, ONBOARDING_COMPLETED_KEY } from "./views/OnboardingView";
import { mountSettingsView } from "./views/SettingsView";
import { mountWheelView } from "./views/WheelView";

// =========================================================================
// Lightweight In-Memory DOM + SVG Shim (cleanly restores all globals)
// =========================================================================

type ListenerEntry = {
  fn: (ev: any) => void;
  capture: boolean;
};

class ShimEvent {
  type: string;
  bubbles: boolean;
  cancelable: boolean;
  defaultPrevented = false;
  propagationStopped = false;
  immediatePropagationStopped = false;
  target: any = null;
  currentTarget: any = null;

  constructor(type: string, init: { bubbles?: boolean; cancelable?: boolean } = {}) {
    this.type = type;
    this.bubbles = Boolean(init.bubbles);
    this.cancelable = Boolean(init.cancelable);
  }

  preventDefault(): void {
    if (this.cancelable) this.defaultPrevented = true;
  }

  stopPropagation(): void {
    this.propagationStopped = true;
  }

  stopImmediatePropagation(): void {
    this.propagationStopped = true;
    this.immediatePropagationStopped = true;
  }
}

class ShimMouseEvent extends ShimEvent {
  clientX: number;
  clientY: number;
  detail: number;
  button: number;
  shiftKey: boolean;
  altKey: boolean;
  ctrlKey: boolean;
  metaKey: boolean;

  constructor(
    type: string,
    init: {
      bubbles?: boolean;
      cancelable?: boolean;
      clientX?: number;
      clientY?: number;
      detail?: number;
      button?: number;
      shiftKey?: boolean;
      altKey?: boolean;
      ctrlKey?: boolean;
      metaKey?: boolean;
    } = {},
  ) {
    super(type, init);
    this.clientX = init.clientX ?? 0;
    this.clientY = init.clientY ?? 0;
    this.detail = init.detail ?? 1;
    this.button = init.button ?? 0;
    this.shiftKey = Boolean(init.shiftKey);
    this.altKey = Boolean(init.altKey);
    this.ctrlKey = Boolean(init.ctrlKey);
    this.metaKey = Boolean(init.metaKey);
  }
}

class ShimPointerEvent extends ShimMouseEvent {
  pointerId: number;
  constructor(
    type: string,
    init: ConstructorParameters<typeof ShimMouseEvent>[1] & { pointerId?: number } = {},
  ) {
    super(type, init);
    this.pointerId = init.pointerId ?? 1;
  }
}

class ShimKeyboardEvent extends ShimEvent {
  key: string;
  code: string;
  shiftKey: boolean;
  altKey: boolean;
  ctrlKey: boolean;
  metaKey: boolean;
  repeat: boolean;

  constructor(
    type: string,
    init: {
      bubbles?: boolean;
      cancelable?: boolean;
      key?: string;
      code?: string;
      shiftKey?: boolean;
      altKey?: boolean;
      ctrlKey?: boolean;
      metaKey?: boolean;
      repeat?: boolean;
    } = {},
  ) {
    super(type, { bubbles: init.bubbles ?? true, cancelable: init.cancelable ?? true });
    this.key = init.key ?? "";
    this.code = init.code ?? "";
    this.shiftKey = Boolean(init.shiftKey);
    this.altKey = Boolean(init.altKey);
    this.ctrlKey = Boolean(init.ctrlKey);
    this.metaKey = Boolean(init.metaKey);
    this.repeat = Boolean(init.repeat);
  }
}

class ShimFocusEvent extends ShimEvent {
  relatedTarget: any;
  constructor(type: string, init: { bubbles?: boolean; relatedTarget?: any } = {}) {
    super(type, init);
    this.relatedTarget = init.relatedTarget ?? null;
  }
}

class ShimDragEvent extends ShimMouseEvent {
  dataTransfer: any;
  constructor(
    type: string,
    init: ConstructorParameters<typeof ShimMouseEvent>[1] & { dataTransfer?: any } = {},
  ) {
    super(type, init);
    this.dataTransfer = init.dataTransfer ?? { files: [] };
  }
}

class ShimEventTarget {
  protected _listeners = new Map<string, ListenerEntry[]>();

  addEventListener(type: string, fn: ((ev: any) => void) | null, options?: boolean | { capture?: boolean }): void {
    if (!fn) return;
    const capture = typeof options === "boolean" ? options : Boolean(options?.capture);
    const list = this._listeners.get(type) ?? [];
    if (!list.some((e) => e.fn === fn && e.capture === capture)) {
      list.push({ fn, capture });
      this._listeners.set(type, list);
    }
  }

  removeEventListener(type: string, fn: ((ev: any) => void) | null, options?: boolean | { capture?: boolean }): void {
    if (!fn) return;
    const capture = typeof options === "boolean" ? options : Boolean(options?.capture);
    const list = this._listeners.get(type);
    if (!list) return;
    this._listeners.set(
      type,
      list.filter((e) => !(e.fn === fn && e.capture === capture)),
    );
  }

  _firePhase(ev: any, capturePhase: boolean): void {
    const entries = [...(this._listeners.get(ev.type) ?? [])];
    for (const entry of entries) {
      if (entry.capture !== capturePhase) continue;
      ev.currentTarget = this;
      entry.fn.call(this, ev);
      if (ev.immediatePropagationStopped) break;
    }
  }

  dispatchEvent(ev: any): boolean {
    ev.target = this;
    this._firePhase(ev, true);
    if (!ev.propagationStopped) {
      this._firePhase(ev, false);
    }
    return !ev.defaultPrevented;
  }
}

class ShimNode extends ShimEventTarget {
  nodeType: number;
  parentNode: ShimNode | null = null;
  childNodes: ShimNode[] = [];
  ownerDocument: ShimDocument;

  constructor(nodeType: number, ownerDocument: ShimDocument) {
    super();
    this.nodeType = nodeType;
    this.ownerDocument = ownerDocument;
  }

  appendChild<T extends ShimNode>(child: T): T {
    if (child.parentNode) {
      child.parentNode.removeChild(child);
    }
    child.parentNode = this;
    this.childNodes.push(child);
    return child;
  }

  removeChild<T extends ShimNode>(child: T): T {
    const idx = this.childNodes.indexOf(child);
    if (idx >= 0) {
      this.childNodes.splice(idx, 1);
      child.parentNode = null;
    }
    return child;
  }

  contains(other: ShimNode | null): boolean {
    let cur: ShimNode | null = other;
    while (cur) {
      if (cur === this) return true;
      cur = cur.parentNode;
    }
    return false;
  }

  get textContent(): string {
    return this.childNodes.map((c) => c.textContent).join("");
  }

  set textContent(val: string) {
    for (const c of this.childNodes) c.parentNode = null;
    this.childNodes = [];
    if (val) {
      this.appendChild(new ShimText(String(val), this.ownerDocument));
    }
  }

  override dispatchEvent(ev: any): boolean {
    ev.target = this;
    const path: ShimEventTarget[] = [];
    let cur: ShimNode | null = this;
    while (cur) {
      path.push(cur);
      cur = cur.parentNode;
    }
    const win = this.ownerDocument.defaultView;
    const isConnectedToDoc = path[path.length - 1] === this.ownerDocument;
    const fullChain: ShimEventTarget[] = isConnectedToDoc && win ? [...path, win] : path;

    // Capture phase: outermost down to target (excluding target itself for capture? DOM includes target in capture/bubble order)
    for (let idx = fullChain.length - 1; idx >= 0; idx--) {
      fullChain[idx]._firePhase(ev, true);
      if (ev.propagationStopped) return !ev.defaultPrevented;
    }
    // Target + bubble phase
    fullChain[0]._firePhase(ev, false);
    if (ev.bubbles && !ev.propagationStopped) {
      for (let idx = 1; idx < fullChain.length; idx++) {
        fullChain[idx]._firePhase(ev, false);
        if (ev.propagationStopped) break;
      }
    }
    return !ev.defaultPrevented;
  }
}

class ShimText extends ShimNode {
  data: string;
  constructor(data: string, ownerDocument: ShimDocument) {
    super(3, ownerDocument);
    this.data = data;
  }
  override get textContent(): string {
    return this.data;
  }
  override set textContent(val: string) {
    this.data = String(val ?? "");
  }
}

const VOID_HTML_TAGS = new Set([
  "area",
  "base",
  "br",
  "col",
  "embed",
  "hr",
  "img",
  "input",
  "link",
  "meta",
  "param",
  "source",
  "track",
  "wbr",
  "path",
  "circle",
  "line",
  "rect",
  "polygon",
  "polyline",
  "stop",
]);

function decodeEntities(s: string): string {
  return s
    .replace(/&amp;/g, "&")
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&quot;/g, '"')
    .replace(/&#39;/g, "'")
    .replace(/&nbsp;/g, " ");
}

function kebabToCamel(s: string): string {
  return s.replace(/-([a-z0-9])/g, (_, ch: string) => ch.toUpperCase());
}

function camelToKebab(s: string): string {
  return s.replace(/([A-Z])/g, "-$1").toLowerCase();
}

function splitTopLevelSelectors(input: string, sep: string): string[] {
  const parts: string[] = [];
  let depthBracket = 0;
  let depthParen = 0;
  let quote: string | null = null;
  let start = 0;
  for (let i = 0; i < input.length; i++) {
    const ch = input[i];
    if (quote) {
      if (ch === quote) quote = null;
      continue;
    }
    if (ch === '"' || ch === "'") {
      quote = ch;
    } else if (ch === "[") {
      depthBracket++;
    } else if (ch === "]") {
      depthBracket = Math.max(0, depthBracket - 1);
    } else if (ch === "(") {
      depthParen++;
    } else if (ch === ")") {
      depthParen = Math.max(0, depthParen - 1);
    } else if (depthBracket === 0 && depthParen === 0 && ch === sep) {
      parts.push(input.slice(start, i));
      start = i + 1;
    }
  }
  parts.push(input.slice(start));
  return parts;
}

function matchesSimpleSelector(el: ShimElement, simple: string): boolean {
  let pos = 0;
  const s = simple.trim();
  if (!s) return false;

  // Optional tag name or * at start
  const tagMatch = /^(\*|[a-zA-Z_][a-zA-Z0-9_-]*)/.exec(s);
  if (tagMatch) {
    const want = tagMatch[1].toLowerCase();
    if (want !== "*" && el.tagName.toLowerCase() !== want) {
      return false;
    }
    pos = tagMatch[0].length;
  }

  while (pos < s.length) {
    const ch = s[pos];
    if (ch === "#") {
      pos++;
      const m = /^([a-zA-Z0-9_-]+)/.exec(s.slice(pos));
      if (!m) return false;
      if (el.id !== m[1]) return false;
      pos += m[1].length;
    } else if (ch === ".") {
      pos++;
      const m = /^([a-zA-Z0-9_-]+)/.exec(s.slice(pos));
      if (!m) return false;
      if (!el.classList.contains(m[1])) return false;
      pos += m[1].length;
    } else if (ch === "[") {
      const close = s.indexOf("]", pos);
      if (close < 0) return false;
      const body = s.slice(pos + 1, close).trim();
      pos = close + 1;
      const eq = body.indexOf("=");
      if (eq < 0) {
        if (!el.hasAttribute(body.toLowerCase())) return false;
      } else {
        const attrName = body.slice(0, eq).trim().toLowerCase();
        let expected = body.slice(eq + 1).trim();
        if (
          (expected.startsWith('"') && expected.endsWith('"')) ||
          (expected.startsWith("'") && expected.endsWith("'"))
        ) {
          expected = expected.slice(1, -1);
        }
        const actual = el.getAttribute(attrName);
        if (actual !== expected) return false;
      }
    } else if (ch === ":") {
      if (s.slice(pos).startsWith(":not(")) {
        let depth = 1;
        let p = pos + 5;
        while (p < s.length && depth > 0) {
          if (s[p] === "(") depth++;
          else if (s[p] === ")") depth--;
          p++;
        }
        const inner = s.slice(pos + 5, p - 1);
        if (matchesSimpleSelector(el, inner)) return false;
        pos = p;
      } else {
        const m = /^:([a-zA-Z-]+)/.exec(s.slice(pos));
        if (!m) return false;
        const pseudo = m[1].toLowerCase();
        pos += m[0].length;
        if (pseudo === "disabled") {
          if (!el.hasAttribute("disabled") && !(el as any).disabled) return false;
        } else if (pseudo === "enabled") {
          if (el.hasAttribute("disabled") || Boolean((el as any).disabled)) return false;
        } else if (pseudo === "checked") {
          if (!(el as any).checked) return false;
        }
      }
    } else {
      return false;
    }
  }
  return true;
}

function tokenizeComplexSelector(sel: string): Array<{ comb: "desc" | "child" | null; simple: string }> {
  const tokens: Array<{ comb: "desc" | "child" | null; simple: string }> = [];
  let i = 0;
  const s = sel.trim();
  let pendingComb: "desc" | "child" | null = null;
  while (i < s.length) {
    if (/\s/.test(s[i])) {
      while (i < s.length && /\s/.test(s[i])) i++;
      if (pendingComb === null && tokens.length > 0) pendingComb = "desc";
      continue;
    }
    if (s[i] === ">") {
      pendingComb = "child";
      i++;
      while (i < s.length && /\s/.test(s[i])) i++;
      continue;
    }
    let depthBracket = 0;
    let depthParen = 0;
    let quote: string | null = null;
    const start = i;
    while (i < s.length) {
      const ch = s[i];
      if (quote) {
        if (ch === quote) quote = null;
        i++;
        continue;
      }
      if (ch === '"' || ch === "'") quote = ch;
      else if (ch === "[") depthBracket++;
      else if (ch === "]") depthBracket = Math.max(0, depthBracket - 1);
      else if (ch === "(") depthParen++;
      else if (ch === ")") depthParen = Math.max(0, depthParen - 1);
      else if (depthBracket === 0 && depthParen === 0 && (/\s/.test(ch) || ch === ">")) {
        break;
      }
      i++;
    }
    tokens.push({ comb: pendingComb, simple: s.slice(start, i) });
    pendingComb = null;
  }
  return tokens;
}

function matchesSelector(el: ShimElement, selectorList: string): boolean {
  const parts = splitTopLevelSelectors(selectorList, ",");
  for (const part of parts) {
    const chain = tokenizeComplexSelector(part);
    if (chain.length === 0) continue;
    if (!matchesSimpleSelector(el, chain[chain.length - 1].simple)) continue;

    let ok = true;
    let cur: ShimElement = el;
    for (let k = chain.length - 2; k >= 0; k--) {
      const comb = chain[k + 1].comb;
      const targetSimple = chain[k].simple;
      if (comb === "child") {
        const p = cur.parentNode;
        if (!(p instanceof ShimElement) || !matchesSimpleSelector(p, targetSimple)) {
          ok = false;
          break;
        }
        cur = p;
      } else {
        let p = cur.parentNode;
        let found: ShimElement | null = null;
        while (p) {
          if (p instanceof ShimElement && matchesSimpleSelector(p, targetSimple)) {
            found = p;
            break;
          }
          p = p.parentNode;
        }
        if (!found) {
          ok = false;
          break;
        }
        cur = found;
      }
    }
    if (ok) return true;
  }
  return false;
}

class ShimClassList {
  private el: ShimElement;
  constructor(el: ShimElement) {
    this.el = el;
  }
  private getSet(): Set<string> {
    const raw = this.el.getAttribute("class") ?? "";
    return new Set(raw.split(/\s+/).filter(Boolean));
  }
  private commit(set: Set<string>): void {
    this.el.setAttribute("class", Array.from(set).join(" "));
  }
  add(...tokens: string[]): void {
    const set = this.getSet();
    for (const tok of tokens) {
      if (tok) set.add(tok);
    }
    this.commit(set);
  }
  remove(...tokens: string[]): void {
    const set = this.getSet();
    for (const tok of tokens) set.delete(tok);
    this.commit(set);
  }
  contains(tok: string): boolean {
    return this.getSet().has(tok);
  }
  toggle(tok: string, force?: boolean): boolean {
    const set = this.getSet();
    const shouldAdd = force === undefined ? !set.has(tok) : force;
    if (shouldAdd) set.add(tok);
    else set.delete(tok);
    this.commit(set);
    return shouldAdd;
  }
}

class ShimElement extends ShimNode {
  tagName: string;
  namespaceURI: string;
  protected _attrs = new Map<string, string>();
  classList: ShimClassList;
  style: Record<string, string>;
  dataset: Record<string, string>;
  tabIndex = -1;

  constructor(tagName: string, namespaceURI: string, ownerDocument: ShimDocument) {
    super(1, ownerDocument);
    this.namespaceURI = namespaceURI;
    this.tagName = namespaceURI.includes("svg") ? tagName : tagName.toUpperCase();
    this.classList = new ShimClassList(this);

    const styleStore: Record<string, string> = {};
    const setStyleProp = (prop: string, value: string) => {
      styleStore[prop] = String(value ?? "");
    };
    const getStyleProp = (prop: string) => styleStore[prop] ?? "";
    this.style = new Proxy(styleStore, {
      get: (target, prop: string) => {
        if (prop === "setProperty") return setStyleProp;
        if (prop === "getPropertyValue") return getStyleProp;
        return target[prop] ?? "";
      },
      set: (target, prop: string, value) => {
        target[prop] = String(value ?? "");
        return true;
      },
    });

    this.dataset = new Proxy({} as Record<string, string>, {
      get: (_target, prop: string) => {
        return this.getAttribute(`data-${camelToKebab(String(prop))}`) ?? undefined;
      },
      set: (_target, prop: string, val) => {
        this.setAttribute(`data-${camelToKebab(String(prop))}`, String(val ?? ""));
        return true;
      },
      deleteProperty: (_target, prop: string) => {
        this.removeAttribute(`data-${camelToKebab(String(prop))}`);
        return true;
      },
    }) as Record<string, string>;
  }

  get id(): string {
    return this._attrs.get("id") ?? "";
  }
  set id(v: string) {
    this.setAttribute("id", v);
  }

  get className(): string {
    return this._attrs.get("class") ?? "";
  }
  set className(v: string) {
    this.setAttribute("class", v);
  }

  get title(): string {
    return this._attrs.get("title") ?? "";
  }
  set title(v: string) {
    this.setAttribute("title", v);
  }

  get offsetWidth(): number {
    return 440;
  }

  getAttribute(name: string): string | null {
    return this._attrs.get(name.toLowerCase()) ?? null;
  }

  setAttribute(name: string, val: string): void {
    const key = name.toLowerCase();
    const sVal = String(val);
    this._attrs.set(key, sVal);
    if (key === "style") {
      for (const decl of sVal.split(";")) {
        const colon = decl.indexOf(":");
        if (colon > 0) {
          const prop = kebabToCamel(decl.slice(0, colon).trim());
          const pval = decl.slice(colon + 1).trim();
          this.style[prop] = pval;
        }
      }
    }
  }

  hasAttribute(name: string): boolean {
    return this._attrs.has(name.toLowerCase());
  }

  removeAttribute(name: string): void {
    this._attrs.delete(name.toLowerCase());
  }

  getBoundingClientRect(): {
    left: number;
    top: number;
    right: number;
    bottom: number;
    width: number;
    height: number;
    x: number;
    y: number;
  } {
    return { left: 0, top: 0, right: 440, bottom: 440, width: 440, height: 440, x: 0, y: 0 };
  }

  matches(sel: string): boolean {
    return matchesSelector(this, sel);
  }

  closest(sel: string): ShimElement | null {
    let cur: ShimNode | null = this;
    while (cur) {
      if (cur instanceof ShimElement && cur.matches(sel)) return cur;
      cur = cur.parentNode;
    }
    return null;
  }

  querySelectorAll<T extends ShimElement = ShimElement>(sel: string): T[] {
    const found: T[] = [];
    const walk = (node: ShimNode) => {
      for (const child of node.childNodes) {
        if (child instanceof ShimElement) {
          if (child.matches(sel)) found.push(child as T);
          walk(child);
        }
      }
    };
    walk(this);
    return found;
  }

  querySelector<T extends ShimElement = ShimElement>(sel: string): T | null {
    return this.querySelectorAll<T>(sel)[0] ?? null;
  }

  focus(): void {
    const prev = this.ownerDocument.activeElement;
    if (prev === this) return;
    if (prev instanceof ShimElement) {
      prev.dispatchEvent(new ShimFocusEvent("focusout", { bubbles: true, relatedTarget: this }));
    }
    this.ownerDocument.activeElement = this;
    this.dispatchEvent(new ShimFocusEvent("focus", { bubbles: false, relatedTarget: prev }));
    this.dispatchEvent(new ShimFocusEvent("focusin", { bubbles: true, relatedTarget: prev }));
  }

  blur(): void {
    if (this.ownerDocument.activeElement === this) {
      this.ownerDocument.activeElement = this.ownerDocument.body;
      this.dispatchEvent(new ShimFocusEvent("blur", { bubbles: false }));
    }
  }

  click(): void {
    if ((this as any).disabled || this.hasAttribute("disabled")) return;
    this.dispatchEvent(new ShimMouseEvent("click", { bubbles: true, cancelable: true, detail: 0 }));
  }

  get innerHTML(): string {
    return "";
  }

  set innerHTML(html: string) {
    for (const c of this.childNodes) c.parentNode = null;
    this.childNodes = [];
    parseHtmlInto(this, String(html ?? ""), this.namespaceURI, this.ownerDocument);
  }
}

class ShimHTMLElement extends ShimElement {
  get disabled(): boolean {
    return this.hasAttribute("disabled");
  }
  set disabled(v: boolean) {
    if (v) this.setAttribute("disabled", "");
    else this.removeAttribute("disabled");
  }
}

class ShimHTMLButtonElement extends ShimHTMLElement {}

class ShimHTMLInputElement extends ShimHTMLElement {
  private _valueOverride: string | null = null;
  private _checkedOverride: boolean | null = null;
  files: File[] = [];

  get type(): string {
    return (this.getAttribute("type") ?? "text").toLowerCase();
  }
  set type(v: string) {
    this.setAttribute("type", v);
  }

  get value(): string {
    return this._valueOverride ?? this.getAttribute("value") ?? "";
  }
  set value(v: string) {
    this._valueOverride = String(v ?? "");
    this.setAttribute("value", this._valueOverride);
  }

  get checked(): boolean {
    if (this._checkedOverride !== null) return this._checkedOverride;
    return this.hasAttribute("checked");
  }
  set checked(v: boolean) {
    this._checkedOverride = Boolean(v);
    if (this._checkedOverride) this.setAttribute("checked", "");
    else this.removeAttribute("checked");
  }

  override click(): void {
    if (this.disabled) return;
    if (this.type === "checkbox") {
      this.checked = !this.checked;
      this.dispatchEvent(new ShimMouseEvent("click", { bubbles: true, cancelable: true }));
      this.dispatchEvent(new ShimEvent("change", { bubbles: true }));
      return;
    }
    super.click();
  }
}

class ShimHTMLSelectElement extends ShimHTMLElement {
  private _valueOverride: string | null = null;
  get value(): string {
    if (this._valueOverride !== null) return this._valueOverride;
    const opts = this.querySelectorAll("option");
    const selected = opts.find((o) => o.hasAttribute("selected")) ?? opts[0];
    return selected?.getAttribute("value") ?? selected?.textContent ?? "";
  }
  set value(v: string) {
    this._valueOverride = String(v ?? "");
  }
}

class ShimHTMLTextAreaElement extends ShimHTMLElement {
  private _valueOverride: string | null = null;
  get value(): string {
    return this._valueOverride ?? this.textContent;
  }
  set value(v: string) {
    this._valueOverride = String(v ?? "");
    this.textContent = this._valueOverride;
  }
  select(): void {}
}

class ShimSVGElement extends ShimElement {}
class ShimSVGGElement extends ShimSVGElement {}
class ShimSVGSVGElement extends ShimSVGElement {}
class ShimSVGPathElement extends ShimSVGElement {}
class ShimSVGLineElement extends ShimSVGElement {}
class ShimSVGCircleElement extends ShimSVGElement {}

class ShimDocument extends ShimNode {
  documentElement: ShimHTMLElement;
  body: ShimHTMLElement;
  activeElement: ShimElement | null = null;
  defaultView: any = null;

  constructor() {
    super(9, undefined as unknown as ShimDocument);
    this.ownerDocument = this;
    this.documentElement = new ShimHTMLElement("html", "http://www.w3.org/1999/xhtml", this);
    this.body = new ShimHTMLElement("body", "http://www.w3.org/1999/xhtml", this);
    this.documentElement.parentNode = this;
    this.documentElement.appendChild(this.body);
    this.childNodes = [this.documentElement];
    this.activeElement = this.body;
  }

  createElement(tag: string): ShimElement {
    const lower = tag.toLowerCase();
    const ns = "http://www.w3.org/1999/xhtml";
    if (lower === "button") return new ShimHTMLButtonElement(lower, ns, this);
    if (lower === "input") return new ShimHTMLInputElement(lower, ns, this);
    if (lower === "select") return new ShimHTMLSelectElement(lower, ns, this);
    if (lower === "textarea") return new ShimHTMLTextAreaElement(lower, ns, this);
    return new ShimHTMLElement(lower, ns, this);
  }

  createElementNS(ns: string, tag: string): ShimElement {
    const lower = tag.toLowerCase();
    if (ns === "http://www.w3.org/2000/svg") {
      if (lower === "svg") return new ShimSVGSVGElement(lower, ns, this);
      if (lower === "g") return new ShimSVGGElement(lower, ns, this);
      if (lower === "path") return new ShimSVGPathElement(lower, ns, this);
      if (lower === "line") return new ShimSVGLineElement(lower, ns, this);
      if (lower === "circle") return new ShimSVGCircleElement(lower, ns, this);
      return new ShimSVGElement(lower, ns, this);
    }
    return this.createElement(tag);
  }

  querySelector<T extends ShimElement = ShimElement>(sel: string): T | null {
    return this.documentElement.querySelector<T>(sel);
  }

  querySelectorAll<T extends ShimElement = ShimElement>(sel: string): T[] {
    return this.documentElement.querySelectorAll<T>(sel);
  }

  execCommand(commandId: string): boolean {
    return commandId.toLowerCase() === "copy";
  }
}

function parseAttributes(raw: string): Array<[string, string]> {
  const attrs: Array<[string, string]> = [];
  let i = 0;
  const len = raw.length;
  while (i < len) {
    while (i < len && /\s/.test(raw[i])) i++;
    if (i >= len || raw[i] === "/") break;
    const nameStart = i;
    while (i < len && !/[\s=/>]/.test(raw[i])) i++;
    const name = raw.slice(nameStart, i).toLowerCase();
    if (!name) break;
    while (i < len && /\s/.test(raw[i])) i++;
    if (i < len && raw[i] === "=") {
      i++;
      while (i < len && /\s/.test(raw[i])) i++;
      if (i < len && (raw[i] === '"' || raw[i] === "'")) {
        const q = raw[i++];
        const valStart = i;
        while (i < len && raw[i] !== q) i++;
        attrs.push([name, decodeEntities(raw.slice(valStart, i))]);
        if (i < len && raw[i] === q) i++;
      } else {
        const valStart = i;
        while (i < len && !/[\s>]/.test(raw[i])) i++;
        attrs.push([name, decodeEntities(raw.slice(valStart, i))]);
      }
    } else {
      attrs.push([name, ""]);
    }
  }
  return attrs;
}

function parseHtmlInto(
  root: ShimElement,
  html: string,
  rootNs: string,
  doc: ShimDocument,
): void {
  const stack: Array<{ el: ShimElement; ns: string }> = [{ el: root, ns: rootNs }];
  let i = 0;
  const len = html.length;
  while (i < len) {
    const lt = html.indexOf("<", i);
    if (lt < 0) {
      const txt = decodeEntities(html.slice(i));
      if (txt) stack[stack.length - 1].el.appendChild(new ShimText(txt, doc));
      break;
    }
    if (lt > i) {
      const txt = decodeEntities(html.slice(i, lt));
      if (txt) stack[stack.length - 1].el.appendChild(new ShimText(txt, doc));
    }
    if (html.startsWith("<!--", lt)) {
      const end = html.indexOf("-->", lt + 4);
      i = end < 0 ? len : end + 3;
      continue;
    }
    // Find closing '>' respecting quotes
    let gt = lt + 1;
    let quote: string | null = null;
    while (gt < len) {
      const c = html[gt];
      if (quote) {
        if (c === quote) quote = null;
      } else if (c === '"' || c === "'") {
        quote = c;
      } else if (c === ">") {
        break;
      }
      gt++;
    }
    if (gt >= len) break;
    const rawTag = html.slice(lt + 1, gt).trim();
    i = gt + 1;

    if (!rawTag || rawTag.startsWith("!") || rawTag.startsWith("?")) continue;
    if (rawTag.startsWith("/")) {
      const closeName = rawTag.slice(1).trim().toLowerCase();
      for (let k = stack.length - 1; k >= 1; k--) {
        if (stack[k].el.tagName.toLowerCase() === closeName) {
          stack.length = k;
          break;
        }
      }
      continue;
    }

    const selfClosing = rawTag.endsWith("/");
    const cleanTag = selfClosing ? rawTag.slice(0, -1).trim() : rawTag;
    const spIdx = cleanTag.search(/\s/);
    const tagName = (spIdx < 0 ? cleanTag : cleanTag.slice(0, spIdx)).toLowerCase();
    const attrStr = spIdx < 0 ? "" : cleanTag.slice(spIdx + 1);

    const parentEntry = stack[stack.length - 1];
    let childNs = parentEntry.ns;
    if (tagName === "svg") {
      childNs = "http://www.w3.org/2000/svg";
    } else if (parentEntry.el.tagName.toLowerCase() === "foreignobject") {
      childNs = "http://www.w3.org/1999/xhtml";
    }

    const el =
      childNs === "http://www.w3.org/2000/svg"
        ? doc.createElementNS(childNs, tagName)
        : doc.createElement(tagName);
    for (const [k, v] of parseAttributes(attrStr)) {
      el.setAttribute(k, v);
    }
    parentEntry.el.appendChild(el);

    if (!selfClosing && !VOID_HTML_TAGS.has(tagName)) {
      stack.push({ el, ns: childNs });
    }
  }
}

class ShimStorage {
  private map = new Map<string, string>();
  get length(): number {
    return this.map.size;
  }
  getItem(k: string): string | null {
    return this.map.get(String(k)) ?? null;
  }
  setItem(k: string, v: string): void {
    this.map.set(String(k), String(v));
  }
  removeItem(k: string): void {
    this.map.delete(String(k));
  }
  clear(): void {
    this.map.clear();
  }
}

const GLOBAL_KEYS = [
  "window",
  "document",
  "Node",
  "Element",
  "HTMLElement",
  "HTMLButtonElement",
  "HTMLInputElement",
  "HTMLSelectElement",
  "HTMLTextAreaElement",
  "SVGElement",
  "SVGGElement",
  "SVGSVGElement",
  "SVGPathElement",
  "SVGLineElement",
  "SVGCircleElement",
  "Event",
  "MouseEvent",
  "PointerEvent",
  "KeyboardEvent",
  "FocusEvent",
  "DragEvent",
  "localStorage",
  "isTauri",
  "requestAnimationFrame",
  "cancelAnimationFrame",
] as const;

let savedGlobalDescriptors: Record<string, PropertyDescriptor | undefined> = {};

function installDomShim(): { document: ShimDocument; window: any; root: HTMLElement } {
  savedGlobalDescriptors = {};
  for (const key of GLOBAL_KEYS) {
    savedGlobalDescriptors[key] = Object.getOwnPropertyDescriptor(globalThis, key);
  }

  const doc = new ShimDocument();
  const root = doc.createElement("div");
  root.id = "app";
  doc.body.appendChild(root as unknown as ShimNode);

  const winTarget = new ShimEventTarget();
  const win: any = {
    document: doc,
    devicePixelRatio: 2,
    location: { hash: "" },
    setTimeout: globalThis.setTimeout.bind(globalThis),
    clearTimeout: globalThis.clearTimeout.bind(globalThis),
    setInterval: globalThis.setInterval.bind(globalThis),
    clearInterval: globalThis.clearInterval.bind(globalThis),
    // Like a browser, frame timestamps share the `performance.now()` timeline that pointer events use.
    requestAnimationFrame: (cb: (t: number) => void) =>
      globalThis.setTimeout(() => cb(performance.now()), 0),
    cancelAnimationFrame: (id: number) => globalThis.clearTimeout(id),
    addEventListener: winTarget.addEventListener.bind(winTarget),
    removeEventListener: winTarget.removeEventListener.bind(winTarget),
    dispatchEvent: winTarget.dispatchEvent.bind(winTarget),
    _firePhase: winTarget._firePhase.bind(winTarget),
  };
  doc.defaultView = win;

  const assignments: Record<string, unknown> = {
    window: win,
    document: doc,
    Node: ShimNode,
    Element: ShimElement,
    HTMLElement: ShimHTMLElement,
    HTMLButtonElement: ShimHTMLButtonElement,
    HTMLInputElement: ShimHTMLInputElement,
    HTMLSelectElement: ShimHTMLSelectElement,
    HTMLTextAreaElement: ShimHTMLTextAreaElement,
    SVGElement: ShimSVGElement,
    SVGGElement: ShimSVGGElement,
    SVGSVGElement: ShimSVGSVGElement,
    SVGPathElement: ShimSVGPathElement,
    SVGLineElement: ShimSVGLineElement,
    SVGCircleElement: ShimSVGCircleElement,
    Event: ShimEvent,
    MouseEvent: ShimMouseEvent,
    PointerEvent: ShimPointerEvent,
    KeyboardEvent: ShimKeyboardEvent,
    FocusEvent: ShimFocusEvent,
    DragEvent: ShimDragEvent,
    localStorage: new ShimStorage(),
    requestAnimationFrame: win.requestAnimationFrame,
    cancelAnimationFrame: win.cancelAnimationFrame,
  };

  for (const [k, v] of Object.entries(assignments)) {
    Object.defineProperty(globalThis, k, {
      value: v,
      writable: true,
      configurable: true,
    });
  }

  return {
    document: doc,
    window: win,
    root: root as unknown as HTMLElement,
  };
}

function restoreDomShim(): void {
  for (const key of GLOBAL_KEYS) {
    const desc = savedGlobalDescriptors[key];
    if (desc) {
      Object.defineProperty(globalThis, key, desc);
    } else {
      delete (globalThis as Record<string, unknown>)[key];
    }
  }
  savedGlobalDescriptors = {};
}

// =========================================================================
// Tauri Runtime IPC & Event Test Harness
// =========================================================================

function sampleNewFileKinds(): NewFileKindInfo[] {
  return [
    { id: "folder", name: "Folder", extension: null, category: "system", page: 1, available: true, disabledReason: null },
    { id: "txt", name: "Plain Text", extension: "txt", category: "document", page: 1, available: true, disabledReason: null },
    { id: "md", name: "Markdown", extension: "md", category: "document", page: 1, available: true, disabledReason: null },
    { id: "rtf", name: "Rich Text", extension: "rtf", category: "document", page: 1, available: true, disabledReason: null },
    { id: "docx", name: "Word Document", extension: "docx", category: "office", page: 1, available: true, disabledReason: null },
    { id: "json", name: "JSON File", extension: "json", category: "data", page: 1, available: true, disabledReason: null },
    { id: "csv", name: "CSV Table", extension: "csv", category: "data", page: 1, available: true, disabledReason: null },
    { id: "html", name: "HTML Page", extension: "html", category: "document", page: 1, available: true, disabledReason: null },
    { id: "clipboard", name: "From Clipboard", extension: null, category: "system", page: 2, available: true, disabledReason: null },
  ];
}

interface TauriHarness {
  calls: Array<{ cmd: string; args: Record<string, any> }>;
  prefs: AppPreferences;
  wheelState: WheelStatePayload;
  emitEvent: (event: string, payload: unknown) => void;
  setCommandOverride: (
    cmd: string,
    fn: (args: Record<string, any>) => unknown | Promise<unknown>,
  ) => void;
}

/** Controllable `performance.now()`: lets a test state how much time passed between pointer events. */
function installAdvanceableClock(): { advance: (ms: number) => void; restore: () => void } {
  const original = performance.now;
  let skewMs = 0;
  performance.now = () => original.call(performance) + skewMs;
  return {
    advance: (ms: number) => {
      skewMs += ms;
    },
    restore: () => {
      performance.now = original;
    },
  };
}

function installTauriHarness(win: any, initialPrefs?: Partial<AppPreferences>): TauriHarness {
  let nextCallbackId = 1;
  let nextListenerId = 1;
  const callbacks = new Map<number, (payload: unknown) => void>();
  const eventListeners = new Map<string, Map<number, number>>();
  const overrides = new Map<string, (args: Record<string, any>) => unknown | Promise<unknown>>();
  const calls: Array<{ cmd: string; args: Record<string, any> }> = [];

  let prefs: AppPreferences = {
    ...defaultPreferences(),
    ...initialPrefs,
  };

  let wheelState: WheelStatePayload = {
    mode: "convert",
    isDrag: false,
    files: ["/tmp/spec.pdf"],
    compatibleFormats: [
      "PNG",
      "JPG",
      "WEBP",
      "HEIC",
      "TIFF",
      "OPTIMIZE_PDF",
      "TXT",
      "DOCX",
      "MD",
      "HTML",
      "RTF",
      "ODT",
      "EPUB",
      "FB2",
      "RST",
      "ZIP",
    ],
    batchActions: [],
    availabilityMessage: "",
    family: "pdf",
    targetFolder: "/tmp",
    originX: 220,
    originY: 220,
  };

  let permissions: PermissionsStatus = {
    accessibility: false,
    automationFinder: "not_determined",
    inputMonitoring: false,
  };

  const defaultEngines: EngineStatus = {
    ffmpeg: "/opt/homebrew/bin/ffmpeg",
    ffprobe: "/opt/homebrew/bin/ffprobe",
    pandoc: null,
    bridge: "/Applications/FLIP.app/Contents/Resources/flip-macos-bridge",
    pages: false,
    numbers: false,
    keynote: false,
    nativeFrameworks: {
      imageIO: true,
      pdfKit: true,
      visionOcr: true,
      avFoundation: true,
    },
  };

  const defaultQuickActions: QuickActionsStatus = {
    servicesDir: "/Users/demo/Library/Services",
    convertInstalled: true,
    newFileInstalled: true,
    convertPath: "/Users/demo/Library/Services/Convert with FLIP.workflow",
    newFilePath: "/Users/demo/Library/Services/New File with FLIP.workflow",
    cliExecutable: "/Applications/FLIP.app/Contents/MacOS/flip",
    cliSymlinkPath: "/Users/demo/.local/bin/flip",
  };

  const kindsRes: NewFileKindsResponse = {
    kinds: sampleNewFileKinds(),
    templates: [],
    clipboard: { available: false, kind: "none", extension: "txt" },
  };

  const emitEvent = (event: string, payload: unknown) => {
    const map = eventListeners.get(event);
    if (!map) return;
    for (const [id, cbId] of map.entries()) {
      callbacks.get(cbId)?.({ event, id, payload });
    }
  };

  Object.defineProperty(globalThis, "isTauri", {
    value: true,
    writable: true,
    configurable: true,
  });

  win.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
    unregisterListener(event: string, eventId: number) {
      eventListeners.get(event)?.delete(eventId);
    },
  };

  win.__TAURI_INTERNALS__ = {
    metadata: {
      currentWebview: { label: "wheel" },
      currentWindow: { label: "wheel" },
    },
    transformCallback(cb: (payload: unknown) => void) {
      const id = nextCallbackId++;
      callbacks.set(id, cb);
      return id;
    },
    async invoke(cmd: string, args: Record<string, any> = {}) {
      if (cmd === "plugin:event|listen") {
        const id = nextListenerId++;
        const event = String(args.event ?? "");
        const handlerId = Number(args.handler);
        let map = eventListeners.get(event);
        if (!map) {
          map = new Map();
          eventListeners.set(event, map);
        }
        map.set(id, handlerId);
        return id;
      }
      if (cmd === "plugin:event|unlisten") {
        const event = String(args.event ?? "");
        const eventId = Number(args.eventId);
        eventListeners.get(event)?.delete(eventId);
        return null;
      }
      if (cmd === "plugin:event|emit") {
        emitEvent(String(args.event ?? ""), args.payload);
        return null;
      }

      calls.push({ cmd, args });
      const override = overrides.get(cmd);
      if (override) {
        return override(args);
      }

      switch (cmd) {
        case "get_preferences":
          return prefs;
        case "save_preferences":
          prefs = { ...defaultPreferences(), ...args.prefs };
          emitEvent("prefs:changed", prefs);
          return prefs;
        case "get_wheel_state":
          return wheelState;
        case "get_new_file_kinds":
          return kindsRes;
        case "get_engine_status":
          return defaultEngines;
        case "get_permissions_status":
          return permissions;
        case "get_quick_actions_status":
          return defaultQuickActions;
        case "get_recent_conversions":
          return prefs.recentConversions;
        case "get_thumbnail":
          return { ok: false, pngBase64: null, width: null, height: null, error: null };
        case "inspect_files": {
          const paths = (args.paths as string[]) ?? [];
          const hasImg = paths.some((p) => /\.(png|jpe?g|webp|heic|svg|ico)$/i.test(p));
          const hasPdf = paths.every((p) => /\.pdf$/i.test(p));
          const res: InspectFilesResult = hasImg
            ? {
                files: paths,
                compatibleFormats: ["PNG", "JPG", "WEBP", "HEIC", "PDF", "ICO", "REMOVE_BG"],
                batchActions: paths.length >= 2 ? ["COMBINE_PDF", "ANIMATE_GIF", "ZIP"] : ["ZIP"],
                availabilityMessage: "",
                family: "image",
              }
            : hasPdf
              ? {
                  files: paths,
                  compatibleFormats: [
                    "PNG",
                    "JPG",
                    "WEBP",
                    "HEIC",
                    "TIFF",
                    "OPTIMIZE_PDF",
                    "TXT",
                    "DOCX",
                    "MD",
                    "HTML",
                    "RTF",
                    "ODT",
                    "EPUB",
                    "FB2",
                    "RST",
                  ],
                  batchActions: paths.length >= 2 ? ["MERGE_PDF", "ZIP"] : ["ZIP"],
                  availabilityMessage: "",
                  family: "pdf",
                }
              : {
                  files: paths,
                  compatibleFormats: ["PDF", "DOCX", "MD", "HTML", "TXT", "EPUB", "FB2", "RST"],
                  batchActions: paths.length >= 2 ? ["ZIP"] : [],
                  availabilityMessage: "",
                  family: "document",
                };
          return res;
        }
        case "switch_wheel_mode":
          wheelState = { ...wheelState, mode: args.mode };
          return wheelState;
        case "dismiss_wheel":
        case "open_window":
        case "open_batch_with_files":
        case "play_wheel_feedback":
        case "cancel_conversion":
        case "reveal_in_finder":
        case "reveal_logs":
          return null;
        case "request_permission":
          if (args.kind === "accessibility_prompt") {
            permissions = { ...permissions, accessibility: true };
          } else if (args.kind === "open_automation") {
            permissions = { ...permissions, automationFinder: "granted" };
          }
          return permissions;
        case "execute_conversion": {
          const paths = (args.paths as string[]) ?? [];
          const formats = (args.formats as string[]) ?? [];
          const results = paths.flatMap((src) =>
            formats.map((fmt) => ({
              source: src,
              format: fmt,
              outputs: [`${src}.${fmt.toLowerCase()}`],
              error: null,
            })),
          );
          const payload: ConversionCompletePayload = {
            success: true,
            results,
            error: null,
          };
          return payload;
        }
        case "execute_create_new":
          return {
            path: `/tmp/untitled.${args.kind || "txt"}`,
            kindId: args.kind,
            revealed: true,
            renameTriggered: false,
            fallbackUsed: false,
          };
        case "get_diagnostics_report":
          return "# FLIP Diagnostics Report\nSimulated Error Trace";
        case "copy_diagnostics_to_clipboard":
          return {
            ok: true,
            method: "pbcopy",
            error: null,
            bytes: 128,
            report: "FLIP Diagnostics Report",
          };
        case "install_finder_quick_actions":
          return defaultQuickActions;
        case "uninstall_finder_quick_actions":
          return {
            ...defaultQuickActions,
            convertInstalled: false,
            newFileInstalled: false,
            cliSymlinkPath: args.removeCli ? null : defaultQuickActions.cliSymlinkPath,
          };
        case "check_for_updates": {
          const update: UpdateCheckResult = {
            currentVersion: "0.1.0",
            latestTag: "v0.1.0",
            releaseName: "v0.1.0",
            htmlUrl: "https://github.com/example/flip",
            publishedAt: null,
            body: "",
            updateAvailable: false,
            checkedAtUtc: "2026-11-18T00:00:00Z",
          };
          return update;
        }
        default:
          return null;
      }
    },
  };

  return {
    calls,
    get prefs() {
      return prefs;
    },
    get wheelState() {
      return wheelState;
    },
    emitEvent,
    setCommandOverride(cmd, fn) {
      overrides.set(cmd, fn);
    },
  };
}

async function flushMicrotasks(turns = 3): Promise<void> {
  for (let i = 0; i < turns; i++) {
    await Promise.resolve();
    await new Promise<void>((resolve) => globalThis.setTimeout(resolve, 0));
  }
}

// =========================================================================
// E2E Test Suites for All 4 Views
// =========================================================================

async function withDomEnv(
  run: (ctx: { doc: ShimDocument; win: any; root: HTMLElement; cleanups: Array<() => void> }) => Promise<void>,
): Promise<void> {
  const { document: doc, window: win, root } = installDomShim();
  const cleanups: Array<() => void> = [];
  try {
    await run({ doc, win, root, cleanups });
  } finally {
    try {
      for (const fn of cleanups.splice(0).reverse()) {
        try {
          fn();
        } catch {
          // ensure restoreDomShim always runs
        }
      }
    } finally {
      restoreDomShim();
    }
  }
}

describe("DOM E2E Suite for All 4 FLIP Views", () => {
  // -----------------------------------------------------------------------
  // 1. WheelView E2E
  // -----------------------------------------------------------------------
  test("WheelView: renders Convert & New File sectors, supports click + keyboard nav + More/Back focus restoration + VoiceOver announcer", async () => {
    await withDomEnv(async ({ doc, win, root, cleanups }) => {
    const harness = installTauriHarness(win);
    const dispose = mountWheelView(root);
    cleanups.push(dispose);
    await flushMicrotasks();

    // 16 PDF formats paginate into 3 pages (7 + 6 + 3 items); slot 7 on page 0 is "More ▸"
    const sectorsP1 = Array.from(root.querySelectorAll(".wheel-sector-group"));
    expect(sectorsP1.length).toBe(8);
    expect(root.querySelector("#wheel-page-dots")?.textContent).toContain("1/3");
    const announcer = root.querySelector("#wheel-live-announcer")!;
    expect(announcer.textContent).toContain("16");

    // Keyboard navigation: ArrowRight selects slot 0 and updates VoiceOver announcer, ArrowLeft wraps backward
    win.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight" }));
    await flushMicrotasks();
    expect(root.querySelectorAll(".wheel-sector-group")[0].classList.contains("is-active")).toBe(true);
    expect(announcer.textContent.length).toBeGreaterThan(0);

    win.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowLeft" }));
    await flushMicrotasks();
    expect(root.querySelectorAll(".wheel-sector-group")[7].classList.contains("is-active")).toBe(true);

    // Click More ▸ (without sector focus) -> flips to 2/3 and announces Page 2 of 3
    const moreGroup = sectorsP1[7] as unknown as ShimElement;
    expect(moreGroup.textContent).toContain(t("en", "wheel.moreSlot"));
    moreGroup.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, detail: 0 }));
    await flushMicrotasks();
    expect(root.querySelector("#wheel-page-dots")?.textContent).toContain("2/3");
    expect(announcer.textContent).toContain("Page 2 of 3");

    // Click Back ◂ -> returns to 1/3 and announces Page 1 of 3
    const backNoFocus = Array.from(root.querySelectorAll(".wheel-sector-group")).find((el) =>
      el.textContent.includes(t("en", "wheel.backSlot")),
    ) as unknown as ShimElement;
    expect(Boolean(backNoFocus)).toBe(true);
    backNoFocus.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, detail: 0 }));
    await flushMicrotasks();
    expect(root.querySelector("#wheel-page-dots")?.textContent).toContain("1/3");
    expect(announcer.textContent).toContain("Page 1 of 3");

    // Focus More ▸ sector inside sectorsLayer, click it -> restores focus to an actionable sector on page 2/3
    const moreFocused = Array.from(root.querySelectorAll(".wheel-sector-group")).find((el) =>
      el.textContent.includes(t("en", "wheel.moreSlot")),
    ) as unknown as ShimElement;
    moreFocused.focus();
    moreFocused.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, detail: 0 }));
    await flushMicrotasks();

    expect(root.querySelector("#wheel-page-dots")?.textContent).toContain("2/3");
    const activeAfterMore = doc.activeElement;
    expect(activeAfterMore).not.toBeNull();
    expect(
      root.querySelector("#wheel-sectors-layer")!.contains(activeAfterMore as unknown as Node),
    ).toBe(true);
    expect(announcer.textContent.length).toBeGreaterThan(0);

    // Click Back ◂ while focused -> returns to page 1/3 and retains focus inside sectorsLayer
    const backGroup = Array.from(root.querySelectorAll(".wheel-sector-group")).find((el) =>
      el.textContent.includes(t("en", "wheel.backSlot")),
    ) as unknown as ShimElement;
    expect(Boolean(backGroup)).toBe(true);
    backGroup.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, detail: 0 }));
    await flushMicrotasks();
    expect(root.querySelector("#wheel-page-dots")?.textContent).toContain("1/3");
    expect(
      root.querySelector("#wheel-sectors-layer")!.contains(doc.activeElement as unknown as Node),
    ).toBe(true);

    // Tab switches between Convert and New File mode; New File renders built-in creation kinds
    win.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab" }));
    await flushMicrotasks();
    expect(harness.calls.some((c) => c.cmd === "switch_wheel_mode" && c.args.mode === "new")).toBe(true);
    const newSectors = Array.from(root.querySelectorAll(".wheel-sector-group"));
    expect(newSectors.length).toBeGreaterThan(0);
    expect(newSectors[0].textContent).toContain("Folder");

    // Tab back to Convert, trigger key "1" to run conversion on slot 0
    win.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab" }));
    await flushMicrotasks();
    expect(harness.calls.some((c) => c.cmd === "switch_wheel_mode" && c.args.mode === "convert")).toBe(true);

    let resolveConv!: (v: ConversionCompletePayload) => void;
    harness.setCommandOverride(
      "execute_conversion",
      () => new Promise<ConversionCompletePayload>((resolve) => { resolveConv = resolve; }),
    );

    win.dispatchEvent(new KeyboardEvent("keydown", { key: "1" }));
    await flushMicrotasks();
    expect(root.querySelector<HTMLElement>("#wheel-progress-layer")?.style.display).toBe("block");

    // Progress event updates VoiceOver milestone announcer
    harness.emitEvent("conversion:progress", { percent: 55, label: "55%" });
    await flushMicrotasks();
    expect(announcer.textContent).toContain("50%");

    // Complete successfully -> renders checkmark SVG + completion subtitle (without location suffix for conversions)
    resolveConv({
      success: true,
      results: [{ source: "/tmp/spec.pdf", format: "PNG", outputs: ["/tmp/spec.png"], error: null }],
      error: null,
    });
    await flushMicrotasks();
    expect(root.querySelector(".wheel-check-svg")).not.toBeNull();
    expect(root.querySelector(".wheel-hub-subtitle.is-success")?.textContent).toBe(
      t("en", "wheel.completed"),
    );

    // Multi-image WheelState exposes GIF+ (ANIMATE_GIF) alongside PDF+ and ICO, and updates live on EN -> IT switch
    harness.emitEvent("wheel:state", {
      mode: "convert",
      files: ["/tmp/a.svg", "/tmp/b.png"],
      isDrag: false,
      compatibleFormats: ["PNG", "JPG", "WEBP", "ICO", "PDF"],
      batchActions: ["COMBINE_PDF", "ANIMATE_GIF", "ZIP"],
      availabilityMessage: "",
      family: "image",
      targetFolder: "/tmp",
      originX: 220,
      originY: 220,
    });
    await flushMicrotasks();
    const imgSectorsEn = Array.from(root.querySelectorAll(".wheel-sector-group"));
    expect(imgSectorsEn.some((el) => el.textContent.includes("GIF+"))).toBe(true);
    expect(
      imgSectorsEn.some((el) =>
        (el.getAttribute("aria-label") ?? "").includes(t("en", "wheel.actionAnimateGif")),
      ),
    ).toBe(true);

    harness.emitEvent("prefs:changed", { ...harness.prefs, language: "it" });
    await flushMicrotasks();
    const imgSectorsIt = Array.from(root.querySelectorAll(".wheel-sector-group"));
    expect(
      imgSectorsIt.some((el) =>
        (el.getAttribute("aria-label") ?? "").includes(t("it", "wheel.actionAnimateGif")),
      ),
    ).toBe(true);

    // Escape dismisses wheel (triggers closing animation then dismiss_wheel)
    win.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    await flushMicrotasks();
    expect(root.querySelector("#wheel-shell")?.classList.contains("is-closing")).toBe(true);
    await new Promise<void>((r) => globalThis.setTimeout(r, 130));
    await flushMicrotasks();
    expect(harness.calls.some((c) => c.cmd === "dismiss_wheel" && c.args.reason === "escape-key")).toBe(
      true,
    );
    });
  });

  test("WheelView: conversion failure renders Error Hub with Copy Diagnostics and Batch Details actions", async () => {
    await withDomEnv(async ({ win, root, cleanups }) => {
    const harness = installTauriHarness(win);
    harness.setCommandOverride("execute_conversion", async () => ({
      success: false,
      results: [
        {
          source: "/tmp/spec.pdf",
          format: "DOCX",
          outputs: [],
          error: "Simulated bridge conversion error",
        },
      ],
      error: "Simulated bridge conversion error",
    }));

    const dispose = mountWheelView(root);
    cleanups.push(dispose);
    await flushMicrotasks();

    // Activate slot 0 via SVG click
    const firstSector = root.querySelector(".wheel-sector-group[data-slot-index=\"0\"]") as unknown as ShimElement;
    firstSector.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, detail: 0 }));
    await flushMicrotasks();

    expect(root.querySelector("#wheel-hub-pct")).toBeNull();
    expect(root.querySelector(".wheel-hub-subtitle.is-error")?.textContent).toContain(
      "Simulated bridge conversion error",
    );

    // Copy Diagnostics button fetches diagnostics report and copies cleanly
    const copyDiagBtn = root.querySelector<HTMLButtonElement>("#hub-copy-diag")!;
    expect(copyDiagBtn).not.toBeNull();
    copyDiagBtn.click();
    await flushMicrotasks();
    expect(harness.calls.some((c) => c.cmd === "get_diagnostics_report")).toBe(true);
    expect(root.querySelector("#hub-copy-diag")?.textContent).toContain(t("en", "common.copied"));

    // Batch Details button opens batch window with files + results and dismisses wheel
    const batchDetailsBtn = root.querySelector<HTMLButtonElement>("#hub-batch-details")!;
    expect(batchDetailsBtn).not.toBeNull();
    batchDetailsBtn.click();
    await flushMicrotasks();
    expect(harness.calls.some((c) => c.cmd === "open_window" && c.args.label === "batch")).toBe(true);
    expect(root.querySelector("#wheel-shell")?.classList.contains("is-closing")).toBe(true);
    await new Promise<void>((r) => globalThis.setTimeout(r, 130));
    await flushMicrotasks();
    expect(
      harness.calls.some((c) => c.cmd === "dismiss_wheel" && c.args.reason === "open-batch-details"),
    ).toBe(true);
    });
  });

  test("WheelView: Smart vs Raw Code toggle (#wheel-raw-toggle, R key, and Option+Click) passes rawMode cleanly", async () => {
    await withDomEnv(async ({ win, root, cleanups }) => {
    const harness = installTauriHarness(win);
    const dispose = mountWheelView(root);
    cleanups.push(dispose);
    await flushMicrotasks();

    const rawBtn = root.querySelector<HTMLButtonElement>("#wheel-raw-toggle")!;
    expect(rawBtn).not.toBeNull();
    expect(rawBtn.getAttribute("aria-pressed")).toBe("false");
    expect(rawBtn.classList.contains("is-raw")).toBe(false);
    expect(rawBtn.textContent).toContain(t("en", "wheel.mode.smart"));

    // Click #wheel-raw-toggle -> enables Raw Mode and updates button + hub hint
    rawBtn.click();
    await flushMicrotasks();
    expect(rawBtn.getAttribute("aria-pressed")).toBe("true");
    expect(rawBtn.classList.contains("is-raw")).toBe(true);
    expect(rawBtn.textContent).toContain(t("en", "wheel.mode.raw"));
    expect(root.querySelector(".wheel-hub-subtitle")?.textContent).toContain(
      t("en", "wheel.mode.raw"),
    );

    // Trigger slot 0 via key "1" -> passes rawMode: true
    win.dispatchEvent(new KeyboardEvent("keydown", { key: "1" }));
    await flushMicrotasks();
    const firstExec = harness.calls.filter((c) => c.cmd === "execute_conversion").pop();
    expect(firstExec?.args.rawMode).toBe(true);

    // Wait for auto-close or reset state via wheel:state -> resets rawMode to false
    harness.emitEvent("wheel:state", {
      mode: "convert",
      files: ["/tmp/page.html"],
      isDrag: false,
      compatibleFormats: ["MD", "TXT", "PDF"],
      batchActions: [],
      availabilityMessage: "",
      family: "document",
      targetFolder: "/tmp",
      originX: 220,
      originY: 220,
    });
    await flushMicrotasks();
    expect(rawBtn.getAttribute("aria-pressed")).toBe("false");

    // Pressing "r" toggles Raw Mode on; pressing "R" toggles it back off
    win.dispatchEvent(new KeyboardEvent("keydown", { key: "r" }));
    await flushMicrotasks();
    expect(rawBtn.getAttribute("aria-pressed")).toBe("true");

    win.dispatchEvent(new KeyboardEvent("keydown", { key: "R" }));
    await flushMicrotasks();
    expect(rawBtn.getAttribute("aria-pressed")).toBe("false");

    // Switching to New File mode hides #wheel-raw-toggle and ignores R key
    win.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab" }));
    await flushMicrotasks();
    expect(rawBtn.style.display).toBe("none");
    win.dispatchEvent(new KeyboardEvent("keydown", { key: "r" }));
    await flushMicrotasks();
    expect(rawBtn.getAttribute("aria-pressed")).toBe("false");

    // Tab back to Convert mode, then Option+Click (altKey: true) on sector triggers instant conversion with rawMode: true
    win.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab" }));
    await flushMicrotasks();
    expect(rawBtn.style.display).toBe("inline-flex");
    const firstSector = root.querySelector('.wheel-sector-group[data-slot-index="0"]') as unknown as ShimElement;
    firstSector.dispatchEvent(
      new MouseEvent("click", { bubbles: true, cancelable: true, detail: 0, altKey: true }),
    );
    await flushMicrotasks();
    const altExec = harness.calls.filter((c) => c.cmd === "execute_conversion").pop();
    expect(altExec?.args.rawMode).toBe(true);

    // Drag mode without wheel window focus: native wheel:toggle-raw toggles Smart <-> Raw and conversion picks up rawMode
    harness.emitEvent("wheel:state", {
      mode: "convert",
      files: ["/tmp/page.html"],
      isDrag: true,
      compatibleFormats: ["MD", "TXT", "PDF"],
      batchActions: [],
      availabilityMessage: "",
      family: "document",
      targetFolder: "/tmp",
      originX: 220,
      originY: 220,
    });
    await flushMicrotasks();
    expect(rawBtn.getAttribute("aria-pressed")).toBe("false");

    harness.emitEvent("wheel:toggle-raw", {});
    await flushMicrotasks();
    expect(rawBtn.getAttribute("aria-pressed")).toBe("true");
    expect(rawBtn.classList.contains("is-raw")).toBe(true);

    // Toggling twice returns to Smart mode, then back to Raw for conversion check
    harness.emitEvent("wheel:toggle-raw", {});
    await flushMicrotasks();
    expect(rawBtn.getAttribute("aria-pressed")).toBe("false");
    harness.emitEvent("wheel:toggle-raw", {});
    await flushMicrotasks();
    expect(rawBtn.getAttribute("aria-pressed")).toBe("true");

    // Releasing pointer over slot 0 (12 o'clock -> 220, 96) commits conversion with rawMode: true
    harness.emitEvent("wheel:pointer-release", { button: 0, localX: 220, localY: 96 });
    await flushMicrotasks();
    const dragToggleExec = harness.calls.filter((c) => c.cmd === "execute_conversion").pop();
    expect(dragToggleExec?.args.rawMode).toBe(true);

    // Switching to New File mode ignores wheel:toggle-raw safely
    harness.emitEvent("wheel:state", {
      mode: "new",
      files: [],
      isDrag: false,
      compatibleFormats: [],
      batchActions: [],
      availabilityMessage: "",
      family: null,
      targetFolder: "/tmp",
      originX: 220,
      originY: 220,
    });
    await flushMicrotasks();
    expect(rawBtn.getAttribute("aria-pressed")).toBe("false");
    harness.emitEvent("wheel:toggle-raw", {});
    await flushMicrotasks();
    expect(rawBtn.getAttribute("aria-pressed")).toBe("false");
    });
  });

  test("WheelView: Tools face (T key, #wheel-face-toggle, wheel:toggle-face) opens Crop/Rotate/Scale sub-pages and runs a tool token", async () => {
    await withDomEnv(async ({ win, root, cleanups }) => {
    const harness = installTauriHarness(win);
    const dispose = mountWheelView(root);
    cleanups.push(dispose);
    await flushMicrotasks();

    const faceBtn = root.querySelector<HTMLButtonElement>("#wheel-face-toggle")!;
    expect(faceBtn).not.toBeNull();
    const labels = () =>
      Array.from(root.querySelectorAll(".wheel-sector-group")).map(
        (el) => el.querySelector(".wheel-sector-label")?.textContent ?? "",
      );
    const click = async (index: number) => {
      const sector = root.querySelector(
        `.wheel-sector-group[data-slot-index="${index}"]`,
      ) as unknown as ShimElement;
      sector.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, detail: 0 }));
      await flushMicrotasks();
    };

    // A PDF has no image tools: no Tools switch, and T does nothing
    expect(faceBtn.style.display).toBe("none");
    const pdfLabels = labels();
    win.dispatchEvent(new KeyboardEvent("keydown", { key: "t" }));
    await flushMicrotasks();
    expect(labels()).toEqual(pdfLabels);

    // A selection of images offers the Tools face
    harness.emitEvent("wheel:state", {
      mode: "convert",
      files: ["/tmp/photo.jpg"],
      isDrag: false,
      compatibleFormats: ["PNG", "WEBP", "TIFF", "REMOVE_BG"],
      batchActions: [],
      toolActions: [...IMAGE_TOOL_TOKENS],
      availabilityMessage: "",
      family: "image",
      targetFolder: "/tmp",
      originX: 220,
      originY: 220,
    });
    await flushMicrotasks();
    expect(faceBtn.style.display).toBe("inline-flex");
    expect(faceBtn.textContent).toContain(t("en", "wheel.faceTools"));
    expect(faceBtn.getAttribute("aria-pressed")).toBe("false");
    expect(labels()).toContain("PNG");

    // T switches to the Tools face: Crop, Rotate, Scale, Remove BG, Palette, EXIF
    win.dispatchEvent(new KeyboardEvent("keydown", { key: "t" }));
    await flushMicrotasks();
    expect(faceBtn.getAttribute("aria-pressed")).toBe("true");
    expect(faceBtn.classList.contains("is-tools")).toBe(true);
    expect(faceBtn.textContent).toContain(t("en", "wheel.faceFormats"));
    expect(labels()).toEqual([
      t("en", "wheel.toolGroup.crop"),
      t("en", "wheel.toolGroup.rotate"),
      t("en", "wheel.toolGroup.scale"),
      "No BG",
      "Palette",
      "No EXIF",
    ]);

    // Crop opens a sub-page of presets plus an "up" slot
    await click(0);
    expect(labels()).toEqual(["9:16", "4:5", "1:1", "16:9", t("en", "wheel.backSlot")]);
    expect(harness.calls.some((c) => c.cmd === "execute_conversion")).toBe(false);

    // Up returns to the first page of tools
    await click(4);
    expect(labels()[0]).toBe(t("en", "wheel.toolGroup.crop"));
    expect(labels().length).toBe(6);

    // Rotate -> 90° right runs the tool token through the normal conversion path
    await click(1);
    expect(labels()).toEqual(["90° R", "90° L", "180°", "Flip H", "Flip V", t("en", "wheel.backSlot")]);
    await click(0);
    const toolExec = harness.calls.filter((c) => c.cmd === "execute_conversion").pop();
    expect(toolExec?.args.paths).toEqual(["/tmp/photo.jpg"]);
    expect(toolExec?.args.formats).toEqual(["ROTATE_RIGHT"]);

    // A new selection starts again on the Formats face (and wheel:toggle-face flips it back and forth)
    harness.emitEvent("wheel:state", {
      mode: "convert",
      files: ["/tmp/other.png"],
      isDrag: true,
      compatibleFormats: ["JPG", "WEBP"],
      batchActions: [],
      toolActions: [...IMAGE_TOOL_TOKENS],
      availabilityMessage: "",
      family: "image",
      targetFolder: "/tmp",
      originX: 220,
      originY: 220,
    });
    await flushMicrotasks();
    expect(faceBtn.getAttribute("aria-pressed")).toBe("false");
    expect(labels()).toEqual(["JPG", "WEBP"]);
    harness.emitEvent("wheel:toggle-face", {});
    await flushMicrotasks();
    expect(faceBtn.getAttribute("aria-pressed")).toBe("true");
    expect(labels()[0]).toBe(t("en", "wheel.toolGroup.crop"));
    harness.emitEvent("wheel:toggle-face", {});
    await flushMicrotasks();
    expect(labels()).toEqual(["JPG", "WEBP"]);

    // A selection without tools drops back to formats and hides the switch
    harness.emitEvent("wheel:state", {
      mode: "convert",
      files: ["/tmp/page.html"],
      isDrag: false,
      compatibleFormats: ["MD", "TXT"],
      batchActions: [],
      toolActions: [],
      availabilityMessage: "",
      family: "document",
      targetFolder: "/tmp",
      originX: 220,
      originY: 220,
    });
    await flushMicrotasks();
    expect(faceBtn.style.display).toBe("none");
    expect(labels()).toEqual(["MD", "TXT"]);
    });
  });

  test("WheelView: single pointer stream prevents hover flickering, preserves #hub-multi-btn identity in place, and clears hover over controls/outer padding", async () => {
    await withDomEnv(async ({ doc, win, root, cleanups }) => {
    const harness = installTauriHarness(win);
    const dispose = mountWheelView(root);
    cleanups.push(dispose);
    await flushMicrotasks();

    const stageEl = root.querySelector(".wheel-stage") as unknown as ShimElement;
    const svgEl = root.querySelector("#wheel-svg") as unknown as ShimElement;
    const rawBtn = root.querySelector("#wheel-raw-toggle") as unknown as ShimElement;
    const initialMultiBtn = root.querySelector("#hub-multi-btn");
    const initialSubtitleEl = root.querySelector(".wheel-hub-subtitle");
    expect(initialMultiBtn).not.toBeNull();
    expect(initialSubtitleEl).not.toBeNull();

    // 1. Backend wheel:pointer-move inside .wheel-top-bar band (before any DOM pointermove) must NOT activate sector 0
    harness.emitEvent("wheel:pointer-move", { localX: 220, localY: 20 });
    await flushMicrotasks();
    expect(root.querySelector(".wheel-sector-group.is-active")).toBeNull();

    // 2. Hover inside visible ring (r = 120 at 12 o'clock -> sector 0) via svgEl pointermove
    svgEl.dispatchEvent(
      new PointerEvent("pointermove", { bubbles: true, clientX: 220, clientY: 100 }),
    );
    await flushMicrotasks();
    const sectorsAfterS0 = Array.from(root.querySelectorAll(".wheel-sector-group"));
    expect(sectorsAfterS0[0].classList.contains("is-active")).toBe(true);
    expect(root.querySelector("#hub-multi-btn")).toBe(initialMultiBtn);
    expect(root.querySelector(".wheel-hub-subtitle")).toBe(initialSubtitleEl);
    const subtitleS0 = initialSubtitleEl!.textContent;

    // 3. Hover across to sector 2 (r = 120 at 3 o'clock -> x=340, y=220) via stageEl pointermove
    stageEl.dispatchEvent(
      new PointerEvent("pointermove", { bubbles: true, clientX: 340, clientY: 220 }),
    );
    await flushMicrotasks();
    const sectorsAfterS2 = Array.from(root.querySelectorAll(".wheel-sector-group"));
    expect(sectorsAfterS0[0].classList.contains("is-active")).toBe(false);
    expect(sectorsAfterS2[2].classList.contains("is-active")).toBe(true);
    expect(root.querySelector("#hub-multi-btn")).toBe(initialMultiBtn);
    expect(root.querySelector(".wheel-hub-subtitle")).toBe(initialSubtitleEl);
    expect(initialSubtitleEl!.textContent).not.toBe(subtitleS0);

    // 4. Stale/competing backend wheel:pointer-move emitted right after DOM pointermove in idle mode is ignored
    harness.emitEvent("wheel:pointer-move", { localX: 220, localY: 100 });
    await flushMicrotasks();
    expect(sectorsAfterS2[2].classList.contains("is-active")).toBe(true);
    expect(sectorsAfterS2[0].classList.contains("is-active")).toBe(false);

    // 5. Moving cursor over #wheel-raw-toggle / .wheel-top-bar clears activeIndex
    rawBtn.dispatchEvent(
      new PointerEvent("pointermove", { bubbles: true, clientX: 280, clientY: 20 }),
    );
    await flushMicrotasks();
    expect(root.querySelector(".wheel-sector-group.is-active")).toBeNull();
    expect(root.querySelector("#hub-multi-btn")).toBe(initialMultiBtn);

    // Re-activate sector 0 inside ring (r = 120)
    svgEl.dispatchEvent(
      new PointerEvent("pointermove", { bubbles: true, clientX: 220, clientY: 100 }),
    );
    await flushMicrotasks();
    expect(root.querySelectorAll(".wheel-sector-group")[0].classList.contains("is-active")).toBe(true);

    // 6. Moving cursor into outer transparent padding (r = 190 at 3 o'clock -> x=410, y=220) in idle non-drag mode clears activeIndex
    svgEl.dispatchEvent(
      new PointerEvent("pointermove", { bubbles: true, clientX: 410, clientY: 220 }),
    );
    await flushMicrotasks();
    expect(root.querySelector(".wheel-sector-group.is-active")).toBeNull();
    expect(root.querySelector("#hub-multi-btn")).toBe(initialMultiBtn);

    // 7. Pointer-triggered focusin after pointerdown does not hijack activeIndex away from cursor
    svgEl.dispatchEvent(
      new PointerEvent("pointermove", { bubbles: true, clientX: 340, clientY: 220 }),
    );
    svgEl.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, clientX: 340, clientY: 220 }));
    (root.querySelectorAll(".wheel-sector-group")[0] as unknown as ShimElement).focus();
    await flushMicrotasks();
    expect(root.querySelectorAll(".wheel-sector-group")[2].classList.contains("is-active")).toBe(true);
    expect(root.querySelectorAll(".wheel-sector-group")[0].classList.contains("is-active")).toBe(false);

    // 8. In New File mode, hovering across sectors also updates hub subtitle in place without recreating hub DOM
    win.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab" }));
    await flushMicrotasks();
    const newHubTitle = root.querySelector(".wheel-hub-title");
    const newHubSub = root.querySelector(".wheel-hub-subtitle");
    svgEl.dispatchEvent(
      new PointerEvent("pointermove", { bubbles: true, clientX: 220, clientY: 100 }),
    );
    await flushMicrotasks();
    expect(root.querySelector(".wheel-hub-title")).toBe(newHubTitle);
    expect(root.querySelector(".wheel-hub-subtitle")).toBe(newHubSub);
    expect(doc.activeElement).not.toBeNull();
    });
  });

  test("WheelView: Logi Options+ circular pucks react to cursor proximity and keep Finder drag tracking locked to cursor without jumping to top-left", async () => {
    await withDomEnv(async ({ win, root, cleanups }) => {
    const scheduledRafIds: number[] = [];
    const canceledRafIds: number[] = [];
    const pendingRafIds = new Set<number>();
    const origRaf = win.requestAnimationFrame;
    const origCaf = win.cancelAnimationFrame;
    win.requestAnimationFrame = (cb: (t: number) => void) => {
      const id = origRaf((ts: number) => {
        pendingRafIds.delete(id);
        cb(ts);
      });
      scheduledRafIds.push(id);
      pendingRafIds.add(id);
      return id;
    };
    win.cancelAnimationFrame = (id: number) => {
      canceledRafIds.push(id);
      pendingRafIds.delete(id);
      origCaf(id);
    };

    const harness = installTauriHarness(win);
    const dispose = mountWheelView(root);
    cleanups.push(dispose);
    await flushMicrotasks();

    const shellEl = root.querySelector<HTMLElement>("#wheel-shell")!;
    const hubTetherEl = root.querySelector<HTMLElement>("#wheel-hub-tether")!;
    expect(shellEl).not.toBeNull();
    expect(hubTetherEl).not.toBeNull();
    expect(shellEl.classList.contains("is-drag-mode")).toBe(false);
    expect(Number(hubTetherEl.style.opacity || "0")).toBe(0);

    // Open in drag mode centered under cursor (220, 220)
    harness.emitEvent("wheel:state", {
      mode: "convert",
      files: ["/tmp/spec.pdf"],
      isDrag: true,
      compatibleFormats: ["PNG", "JPG", "WEBP", "HEIC", "TIFF", "TXT", "MD", "HTML", "RTF"],
      batchActions: [],
      availabilityMessage: "",
      family: "pdf",
      targetFolder: "/tmp",
      originX: 220,
      originY: 220,
    });
    await flushMicrotasks();

    expect(shellEl.classList.contains("is-drag-mode")).toBe(true);

    // Renders floating circular bubbles (.wheel-bubble-disc) + synced Liquid Glass pucks + Logi outer ear on More slot
    expect(root.querySelectorAll(".wheel-bubble-disc").length).toBe(8);
    expect(root.querySelectorAll("#wheel-glass-layer .wheel-glass-puck").length).toBe(8);
    expect(root.querySelectorAll(".wheel-bubble-ear").length).toBe(1);

    // Move pointer to sector 2 center (3 o'clock -> 344, 220) via authoritative monitor stream
    harness.emitEvent("wheel:pointer-move", { localX: 344, localY: 220 });
    await flushMicrotasks();
    const groups = Array.from(root.querySelectorAll<HTMLElement>(".wheel-sector-group"));
    const pucks = Array.from(root.querySelectorAll<HTMLElement>("#wheel-glass-layer .wheel-glass-puck"));
    expect(groups[2].classList.contains("is-active")).toBe(true);
    expect(groups[7].classList.contains("is-active")).toBe(false);
    expect(pucks[2].classList.contains("is-active")).toBe(true);
    expect(pucks[7].classList.contains("is-active")).toBe(false);
    for (const el of [groups[2], pucks[2]]) {
      expect(Number(el.style.getPropertyValue("--bubble-prox"))).toBeGreaterThan(0.9);
      expect(Number(el.style.getPropertyValue("--bubble-scale"))).toBeGreaterThan(1.0);
      expect(el.style.getPropertyValue("--bubble-tx")).toMatch(/^-?\d+(\.\d+)?px$/);
      expect(el.style.getPropertyValue("--bubble-ty")).toMatch(/^-?\d+(\.\d+)?px$/);
      expect(el.style.getPropertyValue("--light-x")).toMatch(/^-?\d+(\.\d+)?%$/);
      expect(el.style.getPropertyValue("--light-y")).toMatch(/^-?\d+(\.\d+)?%$/);
    }
    expect(Number(groups[6].style.getPropertyValue("--bubble-prox"))).toBe(0);
    expect(Number(pucks[6].style.getPropertyValue("--bubble-prox"))).toBe(0);
    expect(Number(hubTetherEl.style.opacity)).toBeGreaterThan(0);
    expect(hubTetherEl.style.getPropertyValue("--tether-angle")).toBe("90.00deg");

    // Move slightly off-center inside slot 2 (330, 212) to verify magnetic translation pull toward cursor
    harness.emitEvent("wheel:pointer-move", { localX: 330, localY: 212 });
    await flushMicrotasks();
    for (const el of [groups[2], pucks[2]]) {
      expect(Number(el.style.getPropertyValue("--bubble-scale"))).toBeGreaterThan(1.0);
      expect(Number(el.style.getPropertyValue("--bubble-prox"))).toBeGreaterThan(0.9);
      const txRaw = el.style.getPropertyValue("--bubble-tx");
      const tyRaw = el.style.getPropertyValue("--bubble-ty");
      expect(txRaw).toMatch(/^-?\d+(\.\d+)?px$/);
      expect(tyRaw).toMatch(/^-?\d+(\.\d+)?px$/);
      expect(Number.isFinite(Number.parseFloat(txRaw))).toBe(true);
      expect(Number.isFinite(Number.parseFloat(tyRaw))).toBe(true);
      expect(Number.parseFloat(txRaw)).toBeLessThan(0);
      expect(Number.parseFloat(tyRaw)).toBeLessThan(0);
      expect(el.style.getPropertyValue("--light-x")).toMatch(/^-?\d+(\.\d+)?%$/);
      expect(el.style.getPropertyValue("--light-y")).toMatch(/^-?\d+(\.\d+)?%$/);
    }

    // Return to sector 2 center (344, 220) before drag interference & release checks
    harness.emitEvent("wheel:pointer-move", { localX: 344, localY: 220 });
    await flushMicrotasks();

    // Simulate competing macOS WebKit native drag-over/enter/leave events while dragging
    for (let i = 0; i < 6; i++) {
      harness.emitEvent("tauri://drag-over", {
        position: { x: 220, y: 220 },
      });
      harness.emitEvent("tauri://drag-enter", {
        paths: ["/tmp/spec.pdf"],
        position: { x: 215, y: 215 },
      });
    }
    harness.emitEvent("tauri://drag-leave", {});
    await flushMicrotasks();

    // Selection MUST stay locked on sector 2 and NEVER jump to top-left slot 7
    expect(groups[2].classList.contains("is-active")).toBe(true);
    expect(groups[7].classList.contains("is-active")).toBe(false);

    // Repeat KeyDown on ArrowDown during drag must not cycle selection away from cursor
    win.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", repeat: true }));
    await flushMicrotasks();
    expect(groups[2].classList.contains("is-active")).toBe(true);

    // Releasing left mouse button at cursor (344, 220) commits conversion once for slot 2's format
    const slot2Label = groups[2].querySelector(".wheel-sector-label")?.textContent?.trim();
    expect(Boolean(slot2Label)).toBe(true);
    harness.emitEvent("wheel:pointer-release", { button: 0, localX: 344, localY: 220 });
    harness.emitEvent("tauri://drag-drop", {
      paths: ["/tmp/spec.pdf"],
      position: { x: 220, y: 220 },
    });
    await flushMicrotasks();

    const execCalls = harness.calls.filter((c) => c.cmd === "execute_conversion");
    expect(execCalls.length).toBe(1);
    expect(execCalls[0].args.formats).toEqual([slot2Label]);
    expect(Number(hubTetherEl.style.opacity || "0")).toBe(0);

    // Re-emitting wheel:state with isDrag: false removes is-drag-mode and resets hub tether
    harness.emitEvent("wheel:state", {
      mode: "convert",
      files: ["/tmp/spec.pdf"],
      isDrag: false,
      compatibleFormats: ["PNG", "JPG", "WEBP", "HEIC", "TIFF", "TXT", "MD", "HTML", "RTF"],
      batchActions: [],
      availabilityMessage: "",
      family: "pdf",
      targetFolder: "/tmp",
      originX: 220,
      originY: 220,
    });
    await flushMicrotasks();
    expect(shellEl.classList.contains("is-drag-mode")).toBe(false);
    expect(Number(hubTetherEl.style.opacity || "0")).toBe(0);

    // Trigger a live physics rAF handle and verify dispose() cancels it via cancelAnimationFrame without throwing
    harness.emitEvent("wheel:pointer-move", { localX: 344, localY: 220 });
    expect(scheduledRafIds.length).toBeGreaterThan(0);
    expect(pendingRafIds.size).toBeGreaterThan(0);
    let disposeError: unknown = null;
    try {
      dispose();
    } catch (err) {
      disposeError = err;
    }
    expect(disposeError).toBeNull();
    expect(canceledRafIds.length).toBeGreaterThan(0);
    expect(pendingRafIds.size).toBe(0);
    expect(Number(hubTetherEl.style.opacity || "0")).toBe(0);
    });
  });

  test("WheelView: Create New File shows destination folder in success hub subtitle and clears it on next conversion", async () => {
    await withDomEnv(async ({ win, root, cleanups }) => {
    const harness = installTauriHarness(win);
    harness.setCommandOverride("execute_create_new", async (args) => ({
      path: `${args.targetFolder || "/Users/demo/Desktop"}/Untitled.${args.kindId || "md"}`,
      kindId: args.kindId,
      revealed: true,
      renameTriggered: false,
      fallbackUsed: false,
    }));

    const dispose = mountWheelView(root);
    cleanups.push(dispose);
    await flushMicrotasks();

    harness.emitEvent("wheel:state", {
      mode: "new",
      files: [],
      isDrag: false,
      compatibleFormats: [],
      batchActions: [],
      availabilityMessage: "",
      family: null,
      targetFolder: "/Users/demo/Desktop",
      originX: 220,
      originY: 220,
    });
    await flushMicrotasks();

    // Trigger slot 2 (Markdown) via key "3"
    win.dispatchEvent(new KeyboardEvent("keydown", { key: "3" }));
    await flushMicrotasks();

    expect(root.querySelector(".wheel-check-svg")).not.toBeNull();
    expect(root.querySelector(".wheel-hub-title")?.textContent).toBe("Untitled.md");
    const successSub = root.querySelector(".wheel-hub-subtitle.is-success");
    expect(successSub?.textContent).toContain(t("en", "wheel.completed"));
    expect(successSub?.textContent).toContain("Desktop");
    expect(successSub?.getAttribute("title")).toBe("/Users/demo/Desktop/Untitled.md");

    // Reopen in Convert mode and verify successLocationLabel does not leak into conversion success hub
    harness.emitEvent("wheel:state", {
      mode: "convert",
      files: ["/tmp/spec.pdf"],
      isDrag: false,
      compatibleFormats: ["PNG"],
      batchActions: [],
      availabilityMessage: "",
      family: "pdf",
      targetFolder: "/tmp",
      originX: 220,
      originY: 220,
    });
    await flushMicrotasks();

    win.dispatchEvent(new KeyboardEvent("keydown", { key: "1" }));
    await flushMicrotasks();
    expect(root.querySelector(".wheel-hub-subtitle.is-success")?.textContent).toBe(
      t("en", "wheel.completed"),
    );
    });
  });

  test("WheelView: Create New File with renameTriggered=true shows success hub cleanly and auto-dismisses faster", async () => {
    await withDomEnv(async ({ win, root, cleanups }) => {
    const harness = installTauriHarness(win);
    harness.setCommandOverride("execute_create_new", async (args) => ({
      path: `${args.targetFolder || "/Users/demo/Desktop"}/Untitled.${args.kindId || "txt"}`,
      kindId: args.kindId,
      revealed: true,
      renameTriggered: true,
      fallbackUsed: false,
    }));

    const dispose = mountWheelView(root);
    cleanups.push(dispose);
    await flushMicrotasks();

    harness.emitEvent("wheel:state", {
      mode: "new",
      files: [],
      isDrag: false,
      compatibleFormats: [],
      batchActions: [],
      availabilityMessage: "",
      family: null,
      targetFolder: "/Users/demo/Desktop",
      originX: 220,
      originY: 220,
    });
    await flushMicrotasks();

    win.dispatchEvent(new KeyboardEvent("keydown", { key: "2" }));
    await flushMicrotasks();

    expect(root.querySelector(".wheel-check-svg")).not.toBeNull();
    expect(root.querySelector("#wheel-shell")?.classList.contains("is-closing")).toBe(false);

    // Shorter 120ms autoCloseTimer triggers closing before 420ms, then 100ms dismiss animation calls dismiss_wheel
    await new Promise<void>((r) => globalThis.setTimeout(r, 150));
    await flushMicrotasks();
    expect(root.querySelector("#wheel-shell")?.classList.contains("is-closing")).toBe(true);

    await new Promise<void>((r) => globalThis.setTimeout(r, 130));
    await flushMicrotasks();
    expect(
      harness.calls.some((c) => c.cmd === "dismiss_wheel" && c.args.reason === "create-new-completed"),
    ).toBe(true);
    });
  });

  test("WheelView: reopen via wheel:state resets isHoveringControl and populates WKWebView CSS vars and inline transforms", async () => {
    await withDomEnv(async ({ win, root, cleanups }) => {
    const harness = installTauriHarness(win);
    const dispose = mountWheelView(root);
    cleanups.push(dispose);
    await flushMicrotasks();

    // Hover #wheel-raw-toggle via DOM pointermove -> sets isHoveringControl = true and clears active sector
    const rawBtn = root.querySelector("#wheel-raw-toggle") as unknown as ShimElement;
    rawBtn.dispatchEvent(
      new PointerEvent("pointermove", { bubbles: true, clientX: 280, clientY: 20 }),
    );
    await flushMicrotasks();
    expect(root.querySelector(".wheel-sector-group.is-active")).toBeNull();

    // Reopen via wheel:state in drag mode WITHOUT emitting escape or calling dismiss
    harness.emitEvent("wheel:state", {
      mode: "convert",
      files: ["/tmp/spec.pdf"],
      isDrag: true,
      compatibleFormats: ["PNG", "JPG", "WEBP", "HEIC", "TIFF", "TXT", "MD", "HTML"],
      batchActions: [],
      availabilityMessage: "",
      family: "pdf",
      targetFolder: "/tmp",
      originX: 220,
      originY: 220,
    });
    await flushMicrotasks();

    // Emit wheel:pointer-move at slot 2 center (344, 220)
    harness.emitEvent("wheel:pointer-move", { localX: 344, localY: 220 });
    await flushMicrotasks();

    const groups = Array.from(root.querySelectorAll<HTMLElement>(".wheel-sector-group"));
    const pucks = Array.from(root.querySelectorAll<HTMLElement>("#wheel-glass-layer .wheel-glass-puck"));
    const bodies = Array.from(root.querySelectorAll<HTMLElement>(".wheel-sector-group .wheel-bubble-body"));

    expect(groups[2].classList.contains("is-active")).toBe(true);
    expect(pucks[2].classList.contains("is-active")).toBe(true);
    for (const el of [groups[2], pucks[2]]) {
      expect(Number(el.style.getPropertyValue("--bubble-prox"))).toBeGreaterThan(0.9);
      const rimPct = el.style.getPropertyValue("--prox-rim-pct");
      const iconPct = el.style.getPropertyValue("--prox-icon-pct");
      expect(rimPct).toMatch(/^\d+\.\d%$/);
      expect(iconPct).toMatch(/^\d+\.\d%$/);
      expect(Number.parseFloat(rimPct)).toBeGreaterThan(0);
      expect(Number.parseFloat(iconPct)).toBeGreaterThan(0);
    }
    expect(pucks[2].style.transform).toMatch(/^translate3d\([^)]+\)\s+scale\([^)]+\)$/);
    expect(bodies[2].style.transform).toMatch(/^translate\([^)]+\)\s+scale\([^)]+\)$/);
    });
  });

  test("WheelView: wheel:escape with alreadyHidden=true resets locally without calling dismiss_wheel, while plain escape still dismisses", async () => {
    await withDomEnv(async ({ win, root, cleanups }) => {
    const pendingRafIds = new Set<number>();
    const origRaf = win.requestAnimationFrame;
    const origCaf = win.cancelAnimationFrame;
    win.requestAnimationFrame = (cb: (t: number) => void) => {
      const id = origRaf((ts: number) => {
        pendingRafIds.delete(id);
        cb(ts);
      });
      pendingRafIds.add(id);
      return id;
    };
    win.cancelAnimationFrame = (id: number) => {
      pendingRafIds.delete(id);
      origCaf(id);
    };

    const harness = installTauriHarness(win);
    const dispose = mountWheelView(root);
    cleanups.push(dispose);
    await flushMicrotasks();

    const shellEl = root.querySelector<HTMLElement>("#wheel-shell")!;
    const hubTetherEl = root.querySelector<HTMLElement>("#wheel-hub-tether")!;

    // A pointer event closer than ~2 ms to the previous physics step is folded into the next step, so
    // state that one frame has passed since mount before expecting an immediate reaction.
    const clock = installAdvanceableClock();
    cleanups.push(clock.restore);
    clock.advance(20);

    // Trigger active pointer move so tether opacity > 0 and physics rAF is scheduled
    harness.emitEvent("wheel:pointer-move", { localX: 344, localY: 220 });
    expect(Number(hubTetherEl.style.opacity || "0")).toBeGreaterThan(0);
    expect(pendingRafIds.size).toBeGreaterThan(0);

    // Emit wheel:escape with alreadyHidden: true -> resets locally without scheduling dismiss_wheel
    harness.emitEvent("wheel:escape", { alreadyHidden: true });
    await new Promise<void>((r) => globalThis.setTimeout(r, 150));
    await flushMicrotasks();

    expect(Number(hubTetherEl.style.opacity || "0")).toBe(0);
    expect(pendingRafIds.size).toBe(0);
    expect(shellEl.classList.contains("is-closing")).toBe(false);
    expect(harness.calls.filter((c) => c.cmd === "dismiss_wheel").length).toBe(0);

    // Emitting wheel:escape without alreadyHidden still triggers closing + dismiss_wheel after 100ms
    harness.emitEvent("wheel:escape", {});
    await flushMicrotasks();
    expect(shellEl.classList.contains("is-closing")).toBe(true);
    await new Promise<void>((r) => globalThis.setTimeout(r, 130));
    await flushMicrotasks();

    const dismissCalls = harness.calls.filter((c) => c.cmd === "dismiss_wheel");
    expect(dismissCalls.length).toBe(1);
    expect(dismissCalls[0].args.reason).toBe("escape-event");
    });
  });

  test("WheelView: async getPreferences during applyState preserves glass-puck and sector-group DOM identities and mid-hover springs when slots are unchanged", async () => {
    await withDomEnv(async ({ win, root, cleanups }) => {
    const harness = installTauriHarness(win);
    const dispose = mountWheelView(root);
    cleanups.push(dispose);
    await flushMicrotasks();

    let resolvePrefs!: (prefs: AppPreferences) => void;
    harness.setCommandOverride(
      "get_preferences",
      () => new Promise<AppPreferences>((resolve) => { resolvePrefs = resolve; }),
    );

    // Emit wheel:state with unchanged slots while get_preferences stays pending
    harness.emitEvent("wheel:state", {
      ...harness.wheelState,
      originX: 220,
      originY: 220,
    });

    const pucksBefore = Array.from(root.querySelectorAll<HTMLElement>("#wheel-glass-layer .wheel-glass-puck"));
    const groupsBefore = Array.from(root.querySelectorAll<HTMLElement>(".wheel-sector-group"));
    expect(pucksBefore.length).toBe(8);
    expect(groupsBefore.length).toBe(8);

    // One frame passes since the last physics step (see the clock note in the wheel:escape test).
    const clock = installAdvanceableClock();
    cleanups.push(clock.restore);
    clock.advance(20);

    // Hover slot 2 before deferred get_preferences resolves
    harness.emitEvent("wheel:pointer-move", { localX: 344, localY: 220 });
    expect(groupsBefore[2].classList.contains("is-active")).toBe(true);
    expect(Number(groupsBefore[2].style.getPropertyValue("--bubble-prox"))).toBeGreaterThan(0.9);

    // Resolve deferred get_preferences and flush microtasks
    resolvePrefs(harness.prefs);
    await flushMicrotasks();

    const pucksAfter = Array.from(root.querySelectorAll<HTMLElement>("#wheel-glass-layer .wheel-glass-puck"));
    const groupsAfter = Array.from(root.querySelectorAll<HTMLElement>(".wheel-sector-group"));
    expect(pucksAfter.length).toBe(pucksBefore.length);
    expect(groupsAfter.length).toBe(groupsBefore.length);
    for (let i = 0; i < pucksBefore.length; i++) {
      expect(pucksAfter[i]).toBe(pucksBefore[i]);
      expect(groupsAfter[i]).toBe(groupsBefore[i]);
    }
    expect(groupsAfter[2].classList.contains("is-active")).toBe(true);
    expect(pucksAfter[2].classList.contains("is-active")).toBe(true);
    expect(Number(groupsAfter[2].style.getPropertyValue("--bubble-prox"))).toBeGreaterThan(0.9);
    expect(Number(pucksAfter[2].style.getPropertyValue("--bubble-prox"))).toBeGreaterThan(0.9);
    expect(Number(groupsAfter[2].style.getPropertyValue("--bubble-scale"))).toBeGreaterThan(1.0);
    });
  });

  test("WheelView: back-to-back wheel:pointer-release and tauri://drag-drop on More flips page once without triggering conversion", async () => {
    await withDomEnv(async ({ win, root, cleanups }) => {
    const harness = installTauriHarness(win);
    const dispose = mountWheelView(root);
    cleanups.push(dispose);
    await flushMicrotasks();

    harness.emitEvent("wheel:state", {
      mode: "convert",
      files: ["/tmp/spec.pdf"],
      isDrag: true,
      compatibleFormats: ["PNG", "JPG", "WEBP", "HEIC", "TIFF", "TXT", "MD", "HTML", "RTF", "DOCX"],
      batchActions: [],
      availabilityMessage: "",
      family: "pdf",
      targetFolder: "/tmp",
      originX: 220,
      originY: 220,
    });
    await flushMicrotasks();

    expect(root.querySelector("#wheel-page-dots")?.textContent).toContain("1/2");
    expect(root.querySelectorAll(".wheel-sector-group")[7]?.textContent).toContain(t("en", "wheel.moreSlot"));

    // Move to center of slot 7 (315 deg -> ~132.3, 132.3), then emit back-to-back pointer-release + native drag-drop
    harness.emitEvent("wheel:pointer-move", { localX: 132.3, localY: 132.3 });
    harness.emitEvent("wheel:pointer-release", { button: 0, localX: 132.3, localY: 132.3 });
    harness.emitEvent("tauri://drag-drop", {
      paths: ["/tmp/spec.pdf"],
      position: { x: 132.3, y: 132.3 },
    });
    await flushMicrotasks();

    expect(root.querySelector("#wheel-page-dots")?.textContent).toContain("2/2");
    const page2Groups = Array.from(root.querySelectorAll(".wheel-sector-group"));
    expect(page2Groups.some((el) => el.textContent.includes(t("en", "wheel.backSlot")))).toBe(true);
    expect(harness.calls.filter((c) => c.cmd === "execute_conversion").length).toBe(0);
    });
  });

  // -----------------------------------------------------------------------
  // 2. SettingsView E2E
  // -----------------------------------------------------------------------
  test("SettingsView: switches all 6 tabs, toggles EN/IT live, records shortcuts & Escape cancel, reorders formats with focus retention, and serializes feedback after saveQueue", async () => {
    await withDomEnv(async ({ doc, win, root, cleanups }) => {
    const harness = installTauriHarness(win);
    const dispose = mountSettingsView(root);
    cleanups.push(dispose);
    await flushMicrotasks();

    // Verify all 6 tabs switch cleanly
    const allTabs = ["general", "shortcuts", "formats", "newfile", "engines", "about"] as const;
    for (const tab of allTabs) {
      const btn = root.querySelector<HTMLButtonElement>(`.nav-item[data-tab="${tab}"]`)!;
      expect(btn).not.toBeNull();
      btn.click();
      await flushMicrotasks();
      expect(btn.classList.contains("is-active")).toBe(true);
      expect(root.querySelector("#settings-main")?.textContent.length).toBeGreaterThan(20);
    }

    // Back to General tab -> live EN <-> IT language switch
    root.querySelector<HTMLButtonElement>('.nav-item[data-tab="general"]')!.click();
    await flushMicrotasks();
    expect(root.querySelector('[data-nav-label="general"]')?.textContent).toBe(t("en", "nav.general"));

    const langSelect = root.querySelector<HTMLSelectElement>("#pref-language")!;
    langSelect.value = "it";
    langSelect.dispatchEvent(new Event("change", { bubbles: true }));
    await flushMicrotasks();
    expect(harness.prefs.language).toBe("it");
    expect(root.querySelector('[data-nav-label="general"]')?.textContent).toBe(t("it", "nav.general"));

    // Switch back to English for predictable assertions
    const langSelectIt = root.querySelector<HTMLSelectElement>("#pref-language")!;
    langSelectIt.value = "en";
    langSelectIt.dispatchEvent(new Event("change", { bubbles: true }));
    await flushMicrotasks();
    expect(harness.prefs.language).toBe("en");

    // Verify #btn-test-wheel-feedback strictly waits for in-flight save_preferences (saveQueue)
    let releaseSave!: (saved: AppPreferences) => void;
    const opOrder: string[] = [];
    harness.setCommandOverride("save_preferences", () => {
      opOrder.push("save:start");
      return new Promise<AppPreferences>((resolve) => {
        releaseSave = (saved) => {
          opOrder.push("save:end");
          resolve(saved);
        };
      });
    });
    harness.setCommandOverride("play_wheel_feedback", () => {
      opOrder.push("feedback:play");
      return null;
    });

    const soundChk = root.querySelector<HTMLInputElement>("#pref-soundEnabled")!;
    soundChk.click();
    await flushMicrotasks();
    expect(opOrder).toEqual(["save:start"]);

    const testFeedbackBtn = root.querySelector<HTMLButtonElement>("#btn-test-wheel-feedback")!;
    testFeedbackBtn.click();
    await flushMicrotasks();
    // playWheelFeedback MUST NOT have fired while save_preferences is still pending
    expect(opOrder).toEqual(["save:start"]);

    releaseSave({ ...harness.prefs, soundEnabled: !harness.prefs.soundEnabled });
    await flushMicrotasks();
    expect(opOrder).toEqual(["save:start", "save:end", "feedback:play"]);

    // Restore normal save_preferences behavior for next steps
    harness.setCommandOverride("save_preferences", (args) => ({
      ...defaultPreferences(),
      ...args.prefs,
    }));

    // Shortcuts tab: record Option+Shift+S then test Escape cancel on drag recorder
    root.querySelector<HTMLButtonElement>('.nav-item[data-tab="shortcuts"]')!.click();
    await flushMicrotasks();

    root.querySelector<HTMLButtonElement>("#btn-record-idle")!.click();
    await flushMicrotasks();
    win.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: "s",
        code: "KeyS",
        altKey: true,
        shiftKey: true,
      }),
    );
    await flushMicrotasks();
    expect(
      harness.calls.some(
        (c) => c.cmd === "save_preferences" && c.args.prefs?.idleShortcut === "Option+Shift+S",
      ),
    ).toBe(true);
    expect(root.querySelector("#settings-main")?.textContent).toContain("Option+Shift+S");

    // Start recording drag trigger, then press Escape -> cancels without saving
    const saveCountBeforeEsc = harness.calls.filter((c) => c.cmd === "save_preferences").length;
    root.querySelector<HTMLButtonElement>("#btn-record-drag")!.click();
    await flushMicrotasks();
    win.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", code: "Escape" }));
    await flushMicrotasks();
    const saveCountAfterEsc = harness.calls.filter((c) => c.cmd === "save_preferences").length;
    expect(saveCountAfterEsc).toBe(saveCountBeforeEsc);
    expect(root.querySelector("#settings-main")?.textContent).toContain(
      t("en", "settings.shortcuts.recCancelled"),
    );

    // Formats tab: reorder preferred formats (▶, ◀, Reset) and check focus retention
    root.querySelector<HTMLButtonElement>('.nav-item[data-tab="formats"]')!.click();
    await flushMicrotasks();

    const firstOrderBefore = [...harness.prefs.preferredFormats.image];
    const moveRightBtn = root.querySelector<HTMLButtonElement>('[data-move-fmt="0"][data-dir="1"]')!;
    moveRightBtn.click();
    await flushMicrotasks();

    const latestOrderAfterMove =
      harness.calls
        .filter((c) => c.cmd === "save_preferences")
        .pop()?.args.prefs.preferredFormats.image ?? [];
    expect(latestOrderAfterMove[0]).toBe(firstOrderBefore[1]);
    expect(latestOrderAfterMove[1]).toBe(firstOrderBefore[0]);
    expect(doc.activeElement?.getAttribute("data-move-fmt")).toBe("1");
    expect(doc.activeElement?.getAttribute("data-dir")).toBe("1");

    // Move back left (◀) from index 1 -> index 0
    const moveLeftBtn = root.querySelector<HTMLButtonElement>('[data-move-fmt="1"][data-dir="-1"]')!;
    moveLeftBtn.click();
    await flushMicrotasks();
    const orderAfterMoveBack =
      harness.calls
        .filter((c) => c.cmd === "save_preferences")
        .pop()?.args.prefs.preferredFormats.image ?? [];
    expect(orderAfterMoveBack[0]).toBe(firstOrderBefore[0]);

    // Reset button restores defaults and retains focus on #btn-reset-family-order
    const resetBtn = root.querySelector<HTMLButtonElement>("#btn-reset-family-order")!;
    resetBtn.click();
    await flushMicrotasks();
    expect(doc.activeElement?.id).toBe("btn-reset-family-order");

    // Engines tab: Remove Quick Actions button uninstalls Quick Actions and hides itself
    root.querySelector<HTMLButtonElement>('.nav-item[data-tab="engines"]')!.click();
    await flushMicrotasks();
    const removeQaBtn = root.querySelector<HTMLButtonElement>("#btn-uninstall-quick-actions")!;
    expect(removeQaBtn).not.toBeNull();
    removeQaBtn.click();
    await flushMicrotasks();
    expect(harness.calls.some((c) => c.cmd === "uninstall_finder_quick_actions")).toBe(true);
    expect(root.querySelector("#btn-uninstall-quick-actions")).toBeNull();
    });
  });

  // -----------------------------------------------------------------------
  // 3. BatchView E2E
  // -----------------------------------------------------------------------
  test("BatchView: loads files, Select All / Clear, chooses formats + batch actions (ZIP, REMOVE_BG, OPTIMIZE_PDF, COMBINE_PDF), converts & renders result rows", async () => {
    await withDomEnv(async ({ win, root, cleanups }) => {
    const harness = installTauriHarness(win);
    const dispose = mountBatchView(root);
    cleanups.push(dispose);
    await flushMicrotasks();

    // Load two image files via #batch-path-input + #batch-add-path
    const pathInput = root.querySelector<HTMLInputElement>("#batch-path-input")!;
    const addBtn = root.querySelector<HTMLButtonElement>("#batch-add-path")!;

    // Note mount initially loaded /tmp/spec.pdf via getWheelState fallback -> clear it first
    root.querySelector<HTMLButtonElement>("#batch-clear-files")!.click();
    await flushMicrotasks();

    pathInput.value = "/tmp/hero.png";
    addBtn.click();
    await flushMicrotasks();
    pathInput.value = "/tmp/card.jpg";
    addBtn.click();
    await flushMicrotasks();

    // Verify Select All checks all compatible formats, and Clear unchecks everything and disables Run
    root.querySelector<HTMLButtonElement>("#batch-select-all")!.click();
    await flushMicrotasks();
    const allCompatChks = Array.from(
      root.querySelectorAll<HTMLInputElement>("#batch-format-grid [data-format-chk]"),
    );
    expect(allCompatChks.length).toBeGreaterThan(0);
    expect(allCompatChks.every((c) => c.checked)).toBe(true);

    root.querySelector<HTMLButtonElement>("#batch-select-none")!.click();
    await flushMicrotasks();
    expect(root.querySelector<HTMLButtonElement>("#batch-run-btn")!.disabled).toBe(true);

    // Check REMOVE_BG, COMBINE_PDF, ANIMATE_GIF, ICO, and ZIP checkboxes and run conversion
    expect(root.querySelector("#batch-actions-grid")?.textContent).toContain("Animate into GIF");
    for (const fmt of ["ICO", "REMOVE_BG", "COMBINE_PDF", "ANIMATE_GIF", "ZIP"]) {
      const chk = root.querySelector<HTMLInputElement>(`[data-format-chk="${fmt}"]`);
      expect(chk).not.toBeNull();
      chk!.click();
    }
    await flushMicrotasks();

    const runBtn = root.querySelector<HTMLButtonElement>("#batch-run-btn")!;
    expect(runBtn.disabled).toBe(false);
    runBtn.click();
    await flushMicrotasks();

    const execCall = harness.calls.filter((c) => c.cmd === "execute_conversion").pop();
    expect(Boolean(execCall)).toBe(true);
    expect(execCall!.args.formats).toContain("ICO");
    expect(execCall!.args.formats).toContain("REMOVE_BG");
    expect(execCall!.args.formats).toContain("COMBINE_PDF");
    expect(execCall!.args.formats).toContain("ANIMATE_GIF");
    expect(execCall!.args.formats).toContain("ZIP");
    expect(root.querySelector<HTMLElement>("#batch-results-card")?.style.display).toBe("block");
    expect(root.querySelectorAll(".batch-result-row").length).toBeGreaterThan(0);

    // Also clear and test PDF file -> OPTIMIZE_PDF + ZIP batch conversion
    root.querySelector<HTMLButtonElement>("#batch-clear-files")!.click();
    await flushMicrotasks();
    const pdfInput = root.querySelector<HTMLInputElement>("#batch-path-input")!;
    pdfInput.value = "/tmp/manual.pdf";
    root.querySelector<HTMLButtonElement>("#batch-add-path")!.click();
    await flushMicrotasks();

    root.querySelector<HTMLButtonElement>("#batch-select-none")!.click();
    await flushMicrotasks();
    const optChk = root.querySelector<HTMLInputElement>('[data-format-chk="OPTIMIZE_PDF"]')!;
    const zipChk = root.querySelector<HTMLInputElement>('[data-format-chk="ZIP"]')!;
    optChk.click();
    zipChk.click();
    await flushMicrotasks();

    root.querySelector<HTMLButtonElement>("#batch-run-btn")!.click();
    await flushMicrotasks();
    const pdfExecCall = harness.calls.filter((c) => c.cmd === "execute_conversion").pop();
    expect(pdfExecCall!.args.formats).toContain("OPTIMIZE_PDF");
    expect(pdfExecCall!.args.formats).toContain("ZIP");
    expect(pdfExecCall!.args.rawMode).toBe(false);
    expect(root.querySelectorAll(".batch-result-row").length).toBe(2);

    // Toggling #batch-raw-mode-toggle passes rawMode: true to execute_conversion
    const batchRawToggle = root.querySelector<HTMLInputElement>("#batch-raw-mode-toggle")!;
    expect(batchRawToggle).not.toBeNull();
    expect(batchRawToggle.checked).toBe(false);
    batchRawToggle.click();
    await flushMicrotasks();
    expect(batchRawToggle.checked).toBe(true);

    root.querySelector<HTMLButtonElement>("#batch-run-btn")!.click();
    await flushMicrotasks();
    const rawBatchCall = harness.calls.filter((c) => c.cmd === "execute_conversion").pop();
    expect(rawBatchCall!.args.rawMode).toBe(true);
    });
  });

  // -----------------------------------------------------------------------
  // 4. OnboardingView E2E
  // -----------------------------------------------------------------------
  test("OnboardingView: steps 1 -> 2 -> 3, fires permission requests, and sets completion key on finish", async () => {
    await withDomEnv(async ({ win, root, cleanups }) => {
    const harness = installTauriHarness(win);
    const dispose = mountOnboardingView(root);
    cleanups.push(dispose);
    await flushMicrotasks();

    // Step 0 (step 1 of 3)
    expect(root.querySelectorAll(".step-pip").length).toBe(3);
    expect(root.querySelectorAll(".step-pip")[0].classList.contains("is-active")).toBe(true);
    expect(root.querySelector("#ob-back")).toBeNull();

    // Next -> Step 1 (Shortcut/Idle wheel intro) -> Back -> Next -> Next -> Step 2 (Permissions + Finish)
    root.querySelector<HTMLButtonElement>("#ob-next")!.click();
    await flushMicrotasks();
    expect(root.querySelectorAll(".step-pip")[1].classList.contains("is-active")).toBe(true);

    root.querySelector<HTMLButtonElement>("#ob-back")!.click();
    await flushMicrotasks();
    expect(root.querySelectorAll(".step-pip")[0].classList.contains("is-active")).toBe(true);
    expect(root.querySelector("#ob-back")).toBeNull();

    root.querySelector<HTMLButtonElement>("#ob-next")!.click();
    await flushMicrotasks();
    root.querySelector<HTMLButtonElement>("#ob-next")!.click();
    await flushMicrotasks();
    expect(root.querySelectorAll(".step-pip")[2].classList.contains("is-active")).toBe(true);

    // Step 2: Trigger permission buttons (#ob-grant-ax, #ob-open-ax, #ob-open-auto)
    const grantAx = root.querySelector<HTMLButtonElement>("#ob-grant-ax");
    const openAx = root.querySelector<HTMLButtonElement>("#ob-open-ax");
    const openAuto = root.querySelector<HTMLButtonElement>("#ob-open-auto");
    expect(grantAx).not.toBeNull();
    expect(openAx).not.toBeNull();
    expect(openAuto).not.toBeNull();

    grantAx!.click();
    await flushMicrotasks();
    openAx!.click();
    await flushMicrotasks();
    openAuto!.click();
    await flushMicrotasks();

    const permKinds = harness.calls
      .filter((c) => c.cmd === "request_permission")
      .map((c) => c.args.kind);
    expect(permKinds).toEqual(["accessibility_prompt", "open_accessibility", "open_automation"]);

    // Finish -> sets localStorage key and opens settings window
    const finishBtn = root.querySelector<HTMLButtonElement>("#ob-finish")!;
    expect(finishBtn).not.toBeNull();
    finishBtn.click();
    await flushMicrotasks();

    expect(localStorage.getItem(ONBOARDING_COMPLETED_KEY)).toBe("true");
    expect(harness.calls.some((c) => c.cmd === "open_window" && c.args.label === "settings")).toBe(true);
    });
  });

  // File names are arbitrary user data and error messages echo them back; the views build HTML strings
  // with innerHTML, so every interpolation must be escaped. Breaking out of a text or attribute context
  // would create an element or an `on*` handler, which the shim's HTML parser would materialise.
  const HOSTILE_NAME = `"><img src=x onerror=alert(1)><svg onload=alert(2)>'<b onmouseover=alert(3)>`;

  function findInjectedMarkup(rootEl: unknown): string[] {
    const hits: string[] = [];
    const walk = (node: ShimNode): void => {
      if (node.nodeType === 1) {
        const el = node as ShimElement;
        const tag = el.tagName.toLowerCase();
        if (["script", "iframe", "object", "embed"].includes(tag)) hits.push(`<${tag}>`);
        if (tag === "img" && el.getAttribute("src") === "x") hits.push("<img src=x>");
        for (const name of (el as any)._attrs.keys() as Iterable<string>) {
          if (name.toLowerCase().startsWith("on")) hits.push(`${tag}[${name}]`);
        }
      }
      node.childNodes.forEach(walk);
    };
    walk(rootEl as ShimNode);
    return hits;
  }

  test("BatchView: hostile file names and conversion errors are rendered as text, never as markup", async () => {
    await withDomEnv(async ({ win, root, cleanups }) => {
      const hostilePath = `/tmp/${HOSTILE_NAME}/${HOSTILE_NAME}.png`;
      const harness = installTauriHarness(win);
      harness.setCommandOverride("execute_conversion", async () => ({
        success: false,
        results: [
          { source: hostilePath, format: "JPG", outputs: [], error: `failed: ${HOSTILE_NAME}` },
        ],
        error: `failed: ${HOSTILE_NAME}`,
      }));
      const dispose = mountBatchView(root);
      cleanups.push(dispose);
      await flushMicrotasks();

      root.querySelector<HTMLButtonElement>("#batch-clear-files")!.click();
      await flushMicrotasks();
      root.querySelector<HTMLInputElement>("#batch-path-input")!.value = hostilePath;
      root.querySelector<HTMLButtonElement>("#batch-add-path")!.click();
      await flushMicrotasks();

      const fileList = root.querySelector("#batch-file-list")!;
      expect(fileList.textContent).toContain(HOSTILE_NAME);
      expect(findInjectedMarkup(root)).toEqual([]);

      root.querySelector<HTMLButtonElement>("#batch-select-all")!.click();
      await flushMicrotasks();
      root.querySelector<HTMLButtonElement>("#batch-run-btn")!.click();
      await flushMicrotasks();

      const results = root.querySelector("#batch-results-list")!;
      expect(results.querySelectorAll(".batch-result-row").length).toBeGreaterThan(0);
      expect(results.textContent).toContain(`failed: ${HOSTILE_NAME}`);
      expect(findInjectedMarkup(root)).toEqual([]);
    });
  });

  test("WheelView: a hostile conversion error message is rendered as text in the Error Hub, never as markup", async () => {
    await withDomEnv(async ({ win, root, cleanups }) => {
      const harness = installTauriHarness(win);
      harness.setCommandOverride("execute_conversion", async () => ({
        success: false,
        results: [{ source: "/tmp/spec.pdf", format: "DOCX", outputs: [], error: `failed: ${HOSTILE_NAME}` }],
        error: `failed: ${HOSTILE_NAME}`,
      }));
      const dispose = mountWheelView(root);
      cleanups.push(dispose);
      await flushMicrotasks();

      const firstSector = root.querySelector(
        '.wheel-sector-group[data-slot-index="0"]',
      ) as unknown as ShimElement;
      firstSector.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, detail: 0 }));
      await flushMicrotasks();

      expect(root.querySelector(".wheel-hub-subtitle.is-error")?.textContent).toContain(
        `failed: ${HOSTILE_NAME}`,
      );
      expect(findInjectedMarkup(root)).toEqual([]);
    });
  });
});
