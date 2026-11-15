// Implements a small set of standard terminal query responses for the
// browser-based terminal.
//
// sshx renders the remote PTY in xterm.js in the browser. The vendored
// xterm fork (sshx-xterm) deliberately disables every escape sequence that
// asks the terminal to report back (cursor position, colors, device
// attributes, ...), because answering them blindly causes problems:
//   - without an answer, applications that wait for a reply before
//     rendering (e.g. Go programs using muesli/termenv, as in `gh`,
//     `flyctl`, and other Bubble Tea CLIs) block forever;
//   - with multiple browser tabs open on the same session, every tab would
//     answer, injecting duplicated responses into the remote shell.
//
// This module answers exactly the queries that common CLIs block on, and
// only from a focused, write-capable viewer (callers gate the `send`
// callback), so at most one response per query is emitted and duplicates
// are avoided as much as possible.
//
// Responses are delivered through the normal input path, exactly as if the
// user had typed them.

import type { IDisposable, IParser, ITheme } from "sshx-xterm";

/** Minimal parser-facing shape of an xterm terminal. */
export interface QueryableTerminal {
  parser: IParser;
  buffer: { active: { cursorX: number; cursorY: number } };
}

/** Convert an xterm color string (e.g. "#a1b2c3") to an X11 rgb: spec. */
export function hexToOscRgb(hex: string): string | null {
  const match = /^#(?:[0-9a-f]{3,4}|[0-9a-f]{6}|[0-9a-f]{8})$/i.exec(
    hex.trim(),
  );
  if (!match) return null;
  let h = match[0].slice(1);
  if (h.length <= 4) {
    h = h
      .split("")
      .map((c) => c + c)
      .join("");
  }
  if (h.length === 8) h = h.slice(0, 6); // Drop alpha.

  const channel = (i: number) =>
    (parseInt(h.slice(i, i + 2), 16) * 0x101).toString(16).padStart(4, "0");
  return `rgb:${channel(0)}/${channel(2)}/${channel(4)}`;
}

/** Cursor position report: `CSI row ; col R`, 1-based. */
export function cursorPositionReport(
  cursorX: number,
  cursorY: number,
  privateMode = false,
): string {
  const prefix = privateMode ? "\x1b[?" : "\x1b[";
  return `${prefix}${cursorY + 1};${cursorX + 1}R`;
}

/**
 * Build the OSC color reply for `OSC 10/11/12 ; ?` requests.
 * `specs` is the payload after the identifier, split on ';'
 * (e.g. "?" or "?;?;?" for stacked queries).
 */
export function oscColorReports(
  id: number,
  specs: string,
  theme: ITheme,
): string[] {
  // Slots for a stacked OSC 10 query are foreground; background; cursor.
  // OSC 11/12 start at the corresponding slot.
  const slots: (keyof ITheme)[] = ["foreground", "background", "cursor"];
  const replies: string[] = [];
  const parts = specs.split(";");
  for (let i = 0; i < parts.length; i++) {
    if (parts[i].trim() !== "?") continue;
    const slot = slots[id - 10 + i];
    if (!slot) break;
    // The cursor color defaults to the foreground color.
    const hex =
      theme[slot] ?? (slot === "cursor" ? theme.foreground : undefined);
    const rgb = typeof hex === "string" ? hexToOscRgb(hex) : null;
    if (!rgb) continue;
    replies.push(`\x1b]${id + i};${rgb}\x1b\\`);
  }
  return replies;
}

/** True when every stacked slot is a query (or malformed input we can't answer). */
export function isOscColorQuery(specs: string): boolean {
  return specs.split(";").every((part) => part.trim() === "?");
}

export interface TerminalQueryResponderOptions {
  /** Sends bytes to the remote process, e.g. through the encrypted input path. */
  send: (data: string) => void;
  /** Returns the theme currently in effect. */
  getTheme: () => ITheme;
  /** Return false to suppress all replies, e.g. when this viewer may not write. */
  isActive?: () => boolean;
}

/**
 * Register handlers for the terminal queries that interactive shells and TUI
 * programs block on. Returns a disposable that removes all handlers.
 */
export function installTerminalQueryResponder(
  term: QueryableTerminal,
  { send, getTheme, isActive = () => true }: TerminalQueryResponderOptions,
): IDisposable {
  const reply = (data: string) => {
    if (isActive()) send(data);
  };
  const disposables: IDisposable[] = [];

  // Device status report: CSI 5 n -> CSI 0 n ("terminal OK").
  disposables.push(
    term.parser.registerCsiHandler({ final: "n" }, (params) => {
      const param = params[0];
      const ps = Array.isArray(param) ? param[0] : param;
      if (ps === 5) {
        reply("\x1b[0n");
      } else if (ps === 6) {
        reply(
          cursorPositionReport(
            term.buffer.active.cursorX,
            term.buffer.active.cursorY,
          ),
        );
      } else {
        return false;
      }
      return true;
    }),
  );

  // DECRQM (CSI ? Ps $ p) is used by some programs to probe mode support;
  // leave it alone — the fork returns true without answering, and adding
  // answers is not needed for readline/bubbletea-style apps.

  // Private device status: CSI ? 6 n -> CSI ? row ; col R.
  disposables.push(
    term.parser.registerCsiHandler({ prefix: "?", final: "n" }, (params) => {
      const param = params[0];
      const ps = Array.isArray(param) ? param[0] : param;
      if (ps === 6) {
        reply(
          cursorPositionReport(
            term.buffer.active.cursorX,
            term.buffer.active.cursorY,
            true,
          ),
        );
        return true;
      }
      return false;
    }),
  );

  // Primary device attributes: CSI c / CSI 0 c -> CSI ? 1 ; 2 c (VT100 with AVO).
  disposables.push(
    term.parser.registerCsiHandler({ final: "c" }, (params) => {
      const param = params[0];
      const ps = Array.isArray(param) ? param[0] : param;
      if (ps === undefined || ps === 0) {
        reply("\x1b[?1;2c");
        return true;
      }
      return false;
    }),
  );

  // Secondary device attributes: CSI > c / CSI > 0 c.
  disposables.push(
    term.parser.registerCsiHandler({ prefix: ">", final: "c" }, (params) => {
      const param = params[0];
      const ps = Array.isArray(param) ? param[0] : param;
      if (ps === undefined || ps === 0) {
        // Report as xterm(276) with no extra features, matching the response
        // xterm would have produced.
        reply("\x1b[>0;276;0c");
        return true;
      }
      return false;
    }),
  );

  // OSC 10/11/12 ; ? — query foreground/background/cursor color.
  // Returning false for non-queries lets the built-in handlers apply SETs.
  for (const id of [10, 11, 12] as const) {
    disposables.push(
      term.parser.registerOscHandler(id, (data) => {
        if (!isOscColorQuery(data)) return false;
        for (const report of oscColorReports(id, data, getTheme())) {
          reply(report);
        }
        // Consume the sequence even if colors are missing from the theme,
        // so the built-in query path (which never answers) is not needed.
        return true;
      }),
    );
  }

  return {
    dispose() {
      for (const d of disposables) d.dispose();
    },
  };
}
