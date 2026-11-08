// The connection to bgws-server and everything it has told this page.
// Frames and binary position updates mutate one frame object and bump a
// version, batched to one render per animation frame.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  applyPositions,
  type FrameView,
  type GameEvent,
  type GameInfo,
  type PackInfo,
  type TerrainProgress,
  type Outbound,
  type Question,
  type ScenarioBrief,
  type UnitDetail,
  type Viewer,
  type Viewshed,
} from "./protocol";

export interface Report {
  title: string;
  text: string;
}

export interface ServerState {
  connected: boolean;
  status: string;
  game: GameInfo | null;
  frame: FrameView | null;
  /** The frame shown is a keyframe from the past (the timeline). */
  scrub: boolean;
  /** The live game's time, for the timeline's end. */
  live: number;
  version: number;
  events: GameEvent[];
  questions: Question[];
  playing: boolean;
  speed: number;
  viewer: Viewer;
  report: Report | null;
  unit: UnitDetail | null;
  viewshed: Viewshed | null;
  scenarios: ScenarioBrief[] | null;
  packs: PackInfo[];
  terrain: TerrainProgress | null;
}

const MAX_EVENTS = 3000;

export function useServer() {
  const ws = useRef<WebSocket | null>(null);
  const frame = useRef<FrameView | null>(null);
  const pending = useRef(false);
  const [state, setState] = useState<ServerState>({
    connected: false,
    status: "connecting…",
    game: null,
    frame: null,
    scrub: false,
    live: 0,
    version: 0,
    events: [],
    questions: [],
    playing: false,
    speed: 10,
    viewer: "umpire",
    report: null,
    unit: null,
    viewshed: null,
    scenarios: null,
    packs: [],
    terrain: null,
  });

  const bump = useCallback((patch?: Partial<ServerState>) => {
    if (patch) setState((s) => ({ ...s, ...patch }));
    if (pending.current) return;
    pending.current = true;
    requestAnimationFrame(() => {
      pending.current = false;
      setState((s) => ({ ...s, frame: frame.current, version: s.version + 1 }));
    });
  }, []);

  const send = useCallback((m: Outbound) => {
    if (ws.current?.readyState === WebSocket.OPEN) ws.current.send(JSON.stringify(m));
  }, []);

  useEffect(() => {
    let closed = false;
    let retry: number | undefined;
    const connect = () => {
      const sock = new WebSocket(`${location.protocol === "https:" ? "wss" : "ws"}://${location.host}/ws`);
      sock.binaryType = "arraybuffer";
      ws.current = sock;
      sock.onopen = () => {
        setState((s) => ({ ...s, connected: true, status: "connected" }));
        sock.send(JSON.stringify({ kind: "hello" }));
      };
      sock.onclose = () => {
        setState((s) => ({ ...s, connected: false, status: "disconnected: is bgws-server running? retrying…" }));
        if (!closed) retry = window.setTimeout(connect, 2000);
      };
      sock.onmessage = (m) => {
        if (m.data instanceof ArrayBuffer) {
          if (frame.current) {
            const t = applyPositions(frame.current, m.data);
            if (t != null) {
              setState((s) => (s.live === t && !s.scrub ? s : { ...s, live: Math.max(s.live, t), scrub: false }));
              bump();
            }
          }
          return;
        }
        if (typeof m.data !== "string") return;
        const msg = JSON.parse(m.data);
        switch (msg.kind) {
          case "frame":
            frame.current = msg.frame;
            setState((s) => ({ ...s, scrub: !!msg.scrub, live: msg.live ?? s.live }));
            bump();
            break;
          case "events":
            setState((s) => {
              const events = msg.replay ? msg.events : s.events.concat(msg.events);
              return { ...s, events: events.length > MAX_EVENTS ? events.slice(-MAX_EVENTS) : events };
            });
            break;
          case "questions":
            setState((s) => {
              const same = s.questions.length === msg.questions.length && s.questions.every((q, i) => q.id === msg.questions[i].id);
              return same ? s : { ...s, questions: msg.questions };
            });
            break;
          case "state":
            setState((s) => ({ ...s, playing: msg.playing, speed: msg.speed, viewer: msg.viewer }));
            break;
          case "status":
            setState((s) => ({ ...s, status: msg.text }));
            break;
          case "game":
            setState((s) => ({ ...s, game: msg }));
            break;
          case "branched":
            frame.current = null;
            // A new game drops the old feed; a branch keeps it up to the moment it branched.
            setState((s) => ({ ...s, events: msg.new ? [] : s.events.filter((e) => e.t <= msg.t), questions: [], unit: null, viewshed: null, frame: null, live: msg.t }));
            break;
          case "report":
            setState((s) => ({ ...s, report: { title: msg.title, text: msg.text } }));
            break;
          case "unit":
            setState((s) => ({ ...s, unit: msg }));
            break;
          case "viewshed":
            setState((s) => ({ ...s, viewshed: msg }));
            break;
          case "scenarios":
            setState((s) => ({ ...s, scenarios: msg.scenarios, packs: msg.packs ?? s.packs }));
            break;
          case "terrain":
            setState((s) => ({ ...s, terrain: msg }));
            break;
        }
      };
    };
    connect();
    return () => {
      closed = true;
      window.clearTimeout(retry);
      ws.current?.close();
    };
  }, [bump]);

  const actions = useMemo(
    () => ({
      send,
      setPlaying: (p: boolean) => {
        setState((s) => ({ ...s, playing: p }));
        send({ kind: p ? "play" : "pause" });
      },
      setSpeed: (speed: number) => {
        setState((s) => ({ ...s, speed }));
        send({ kind: "speed", speed });
      },
      setViewer: (viewer: Viewer) => {
        setState((s) => ({ ...s, viewer, unit: null, viewshed: null }));
        send({ kind: "viewer", viewer });
        // The feed again, as this viewer may read it (fog of war).
        send({ kind: "hello" });
      },
      answer: (question: number, choice: string) => {
        setState((s) => ({ ...s, questions: s.questions.filter((q) => q.id !== question) }));
        send({ kind: "answer", question, choice });
      },
      closeReport: () => setState((s) => ({ ...s, report: null })),
      clearUnit: () => setState((s) => ({ ...s, unit: null })),
      clearViewshed: () => setState((s) => ({ ...s, viewshed: null })),
    }),
    [send],
  );

  return { state, ...actions };
}

export type Server = ReturnType<typeof useServer>;
