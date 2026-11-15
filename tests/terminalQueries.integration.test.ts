// @vitest-environment happy-dom

/**
 * Integration test: feeds real escape sequences through the real (patched)
 * xterm parser in sshx-xterm and verifies the responder produces the expected
 * reply bytes through the "input" path. This guards against dispatch-shape
 * mistakes (param layout, handler registration, LIFO ordering).
 */
import { describe, expect, it } from "vitest";

import { Terminal } from "sshx-xterm";

import {
  installTerminalQueryResponder,
  type QueryableTerminal,
} from "../src/lib/terminalQueries";

const theme = {
  foreground: "#d8d8d8",
  background: "#181818",
  cursor: "#ff8800",
};

function makeTerminal() {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const term = new Terminal({ cols: 80, rows: 24 });
  term.open(container);

  // Replies captured through the same path XTerm.svelte would use.
  const sent: string[] = [];
  installTerminalQueryResponder(term as unknown as QueryableTerminal, {
    send: (data) => sent.push(data),
    getTheme: () => theme,
    isActive: () => true,
  });

  const feed = (data: string) =>
    new Promise<void>((resolve) => {
      term.write(data, resolve);
    });

  return { term, sent, feed };
}

describe("terminal query responder (integration with the real parser)", () => {
  it("answers OSC 11 background color query", async () => {
    const { feed, sent } = makeTerminal();
    await feed("\x1b]11;?\x07");
    expect(sent).toEqual(["\x1b]11;rgb:1818/1818/1818\x1b\\"]);
  });

  it("answers OSC 11 query terminated by ST", async () => {
    const { feed, sent } = makeTerminal();
    await feed("\x1b]11;?\x1b\\");
    expect(sent).toEqual(["\x1b]11;rgb:1818/1818/1818\x1b\\"]);
  });

  it("answers OSC 10 and OSC 12 queries", async () => {
    const { feed, sent } = makeTerminal();
    await feed("\x1b]10;?\x07");
    await feed("\x1b]12;?\x07");
    expect(sent).toEqual([
      "\x1b]10;rgb:d8d8/d8d8/d8d8\x1b\\",
      "\x1b]12;rgb:ffff/8888/0000\x1b\\",
    ]);
  });

  it("does not consume OSC color set sequences", async () => {
    const { feed, sent } = makeTerminal();
    await feed("\x1b]11;rgb:1020/3040/5060\x07");
    expect(sent).toEqual([]);
  });

  it("answers DSR 5 and DSR 6 with the cursor position", async () => {
    const { feed, sent } = makeTerminal();
    await feed("\x1b[5n");
    await feed("\x1b[6n");
    expect(sent).toEqual(["\x1b[0n", "\x1b[1;1R"]);
  });

  it("answers DSR 6 after the cursor has moved", async () => {
    const { feed, sent } = makeTerminal();
    await feed("abc\r\ndef"); // cursor at row 2, col 4
    await feed("\x1b[6n");
    expect(sent).toEqual(["\x1b[2;4R"]);
  });

  it("answers private DSR ?6", async () => {
    const { feed, sent } = makeTerminal();
    await feed("\x1b[?6n");
    expect(sent).toEqual(["\x1b[?1;1R"]);
  });

  it("answers primary and secondary device attributes", async () => {
    const { feed, sent } = makeTerminal();
    await feed("\x1b[c");
    await feed("\x1b[0c");
    await feed("\x1b[>c");
    await feed("\x1b[>0c");
    expect(sent).toEqual([
      "\x1b[?1;2c",
      "\x1b[?1;2c",
      "\x1b[>0;276;0c",
      "\x1b[>0;276;0c",
    ]);
  });

  it("answers the exact query sequence emitted by termenv (OSC 11 + DSR 6)", async () => {
    const { feed, sent } = makeTerminal();
    await feed("\x1b]11;?\x1b\\\x1b[6n");
    expect(sent).toEqual(["\x1b]11;rgb:1818/1818/1818\x1b\\", "\x1b[1;1R"]);
  });
});
