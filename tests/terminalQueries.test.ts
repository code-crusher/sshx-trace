import { describe, expect, it } from "vitest";

import type { IFunctionIdentifier } from "sshx-xterm";

import {
  cursorPositionReport,
  hexToOscRgb,
  installTerminalQueryResponder,
  isOscColorQuery,
  oscColorReports,
  type QueryableTerminal,
} from "../src/lib/terminalQueries";

/** Captures registered parser handlers so tests can invoke them directly. */
function makeFakeTerminal(cursorX = 4, cursorY = 9) {
  type CsiCallback = (params: (number | number[])[]) => boolean;
  const csiHandlers = new Map<string, CsiCallback[]>();
  const oscHandlers = new Map<number, ((data: string) => boolean)[]>();

  const term: QueryableTerminal = {
    parser: {
      registerCsiHandler(
        id: IFunctionIdentifier,
        callback: (params: (number | number[])[]) => boolean,
      ) {
        const key = `${id.prefix ?? ""}${
          id.intermediates ? `!${id.intermediates}` : ""
        }${id.final}`;
        const list = csiHandlers.get(key) ?? [];
        const handler = callback;
        list.push(handler);
        csiHandlers.set(key, list);
        return { dispose: () => list.splice(list.indexOf(handler), 1) };
      },
      registerOscHandler(id: number, callback: (data: string) => boolean) {
        const list = oscHandlers.get(id) ?? [];
        const handler = callback as (data: string) => boolean;
        list.push(handler);
        oscHandlers.set(id, list);
        return { dispose: () => list.splice(list.indexOf(handler), 1) };
      },
      // Unused by the responder; satisfy the IParser interface.
      registerDcsHandler: () => {
        throw new Error("not implemented");
      },
      registerEscHandler: () => {
        throw new Error("not implemented");
      },
      registerApcHandler: () => {
        throw new Error("not implemented");
      },
      registerSosHandler: () => {
        throw new Error("not implemented");
      },
      registerPmHandler: () => {
        throw new Error("not implemented");
      },
    } as unknown as QueryableTerminal["parser"],
    buffer: { active: { cursorX, cursorY } },
  };

  const emit = {
    csi(final: string, params: (number | number[])[], prefix?: string) {
      const key = `${prefix ?? ""}${final}`;
      return (csiHandlers.get(key) ?? []).map((h) => h(params));
    },
    osc(id: number, data: string) {
      return (oscHandlers.get(id) ?? []).map((h) => h(data));
    },
  };

  return { term, emit };
}

const theme = {
  foreground: "#d8d8d8",
  background: "#181818",
  cursor: "#ff8800",
};

describe("hexToOscRgb", () => {
  it("converts 6-digit hex colors to xterm rgb: format", () => {
    expect(hexToOscRgb("#000000")).toBe("rgb:0000/0000/0000");
    expect(hexToOscRgb("#ffffff")).toBe("rgb:ffff/ffff/ffff");
    expect(hexToOscRgb("#181818")).toBe("rgb:1818/1818/1818");
    expect(hexToOscRgb("#d8d8d8")).toBe("rgb:d8d8/d8d8/d8d8");
  });

  it("expands 3-digit hex colors", () => {
    expect(hexToOscRgb("#fff")).toBe("rgb:ffff/ffff/ffff");
    expect(hexToOscRgb("#abc")).toBe("rgb:aaaa/bbbb/cccc");
  });

  it("rejects invalid colors", () => {
    expect(hexToOscRgb("red")).toBeNull();
    expect(hexToOscRgb("")).toBeNull();
    expect(hexToOscRgb("#12345")).toBeNull();
    expect(hexToOscRgb("#gggggg")).toBeNull();
  });
});

describe("cursorPositionReport", () => {
  it("reports 1-based positions", () => {
    expect(cursorPositionReport(0, 0)).toBe("\x1b[1;1R");
    expect(cursorPositionReport(4, 9)).toBe("\x1b[10;5R");
  });

  it("supports the private (DECRQM) variant", () => {
    expect(cursorPositionReport(4, 9, true)).toBe("\x1b[?10;5R");
  });
});

describe("isOscColorQuery", () => {
  it("accepts single and stacked queries", () => {
    expect(isOscColorQuery("?")).toBe(true);
    expect(isOscColorQuery("?;?;?")).toBe(true);
  });

  it("rejects setters and mixed input", () => {
    expect(isOscColorQuery("rgb:1111/2222/3333")).toBe(false);
    expect(isOscColorQuery("rgb:1111/2222/3333;?")).toBe(false);
    expect(isOscColorQuery("")).toBe(false);
  });
});

describe("oscColorReports", () => {
  it("reports the foreground color for OSC 10", () => {
    expect(oscColorReports(10, "?", theme)).toEqual([
      "\x1b]10;rgb:d8d8/d8d8/d8d8\x1b\\",
    ]);
  });

  it("reports the background color for OSC 11", () => {
    expect(oscColorReports(11, "?", theme)).toEqual([
      "\x1b]11;rgb:1818/1818/1818\x1b\\",
    ]);
  });

  it("reports the cursor color for OSC 12", () => {
    expect(oscColorReports(12, "?", theme)).toEqual([
      "\x1b]12;rgb:ffff/8888/0000\x1b\\",
    ]);
  });

  it("falls back to the foreground color when the cursor color is unset", () => {
    const rest: { foreground?: string; background?: string; cursor?: string } =
      { ...theme };
    delete rest.cursor;
    expect(oscColorReports(12, "?", rest)).toEqual([
      "\x1b]12;rgb:d8d8/d8d8/d8d8\x1b\\",
    ]);
  });

  it("handles stacked queries starting at the requested slot", () => {
    expect(oscColorReports(10, "?;?;?", theme)).toEqual([
      "\x1b]10;rgb:d8d8/d8d8/d8d8\x1b\\",
      "\x1b]11;rgb:1818/1818/1818\x1b\\",
      "\x1b]12;rgb:ffff/8888/0000\x1b\\",
    ]);
  });

  it("skips non-query slots silently", () => {
    expect(oscColorReports(10, "rgb:0000/0000/0000", theme)).toEqual([]);
  });
});

describe("installTerminalQueryResponder", () => {
  function setup(overrides?: { isActive?: () => boolean }) {
    const { term, emit } = makeFakeTerminal(4, 9);
    const sent: string[] = [];
    const disposable = installTerminalQueryResponder(term, {
      send: (data) => sent.push(data),
      getTheme: () => theme,
      isActive: overrides?.isActive,
    });
    return { term, emit, sent, disposable };
  }

  it("answers DSR 5 with a status report", () => {
    const { emit, sent } = setup();
    expect(emit.csi("n", [5])).toEqual([true]);
    expect(sent).toEqual(["\x1b[0n"]);
  });

  it("answers DSR 6 with the cursor position", () => {
    const { emit, sent } = setup();
    expect(emit.csi("n", [6])).toEqual([true]);
    expect(sent).toEqual(["\x1b[10;5R"]);
  });

  it("answers DECDSR ?6 with the private cursor position", () => {
    const { emit, sent } = setup({ isActive: () => true });
    expect(emit.csi("n", [6], "?")).toEqual([true]);
    expect(sent).toEqual(["\x1b[?10;5R"]);
  });

  it("answers CSI c with primary device attributes", () => {
    const { emit, sent } = setup();
    expect(emit.csi("c", [])).toEqual([true]);
    expect(emit.csi("c", [0])).toEqual([true]);
    expect(sent).toEqual(["\x1b[?1;2c", "\x1b[?1;2c"]);
  });

  it("answers CSI > c with secondary device attributes", () => {
    const { emit, sent } = setup();
    expect(emit.csi("c", [], ">")).toEqual([true]);
    expect(sent).toEqual(["\x1b[>0;276;0c"]);
  });

  it("ignores non-query CSI n parameters", () => {
    const { emit, sent } = setup();
    expect(emit.csi("n", [99])).toEqual([false]);
    expect(sent).toEqual([]);
  });

  it("answers OSC 11 queries with the theme background", () => {
    const { emit, sent } = setup();
    expect(emit.osc(11, "?")).toEqual([true]);
    expect(sent).toEqual(["\x1b]11;rgb:1818/1818/1818\x1b\\"]);
  });

  it("answers stacked OSC 10 queries", () => {
    const { emit, sent } = setup();
    expect(emit.osc(10, "?;?")).toEqual([true]);
    expect(sent).toEqual([
      "\x1b]10;rgb:d8d8/d8d8/d8d8\x1b\\",
      "\x1b]11;rgb:1818/1818/1818\x1b\\",
    ]);
  });

  it("does not consume OSC color setters", () => {
    const { emit, sent } = setup();
    expect(emit.osc(11, "rgb:1020/3040/5060")).toEqual([false]);
    expect(sent).toEqual([]);
  });

  it("sends nothing when the viewer is inactive", () => {
    const { emit, sent } = setup({ isActive: () => false });
    emit.csi("n", [6]);
    emit.osc(11, "?");
    expect(sent).toEqual([]);
  });

  it("removes all handlers on dispose", () => {
    const { emit, sent, disposable } = setup();
    disposable.dispose();
    expect(emit.csi("n", [5])).toEqual([]);
    expect(emit.osc(11, "?")).toEqual([]);
    expect(sent).toEqual([]);
  });
});
