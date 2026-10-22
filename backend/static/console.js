// The Observatory's console (D1.1, plan `the-capitol-devtools.md` §4 D1).
//
// One page, served by the backend it watches. It signs in through
// `/auth/login` and keeps the token in sessionStorage. It reads history from
// `/admin/series` and the live feed from `/admin/stream`. The stream needs the
// bearer header, so it's read with `fetch` and a stream reader: `EventSource`
// can't send one (D0.3). Every chart has a plain sentence over it, and the
// server's annotations drawn as marks across it.
//
// D1.1 is the frame: sign-in, the tabs, the theme, the stream, and one story
// per tab that later packages grow (D1.2 the overview, D2 the economy, D3.1
// performance, D4.2 the town, D6.2 tuning).

"use strict";

(() => {
  const SAMPLE_MS = 10_000;
  const DAY_MS = 24 * 3_600_000;
  const RANGES = [1, 6, 24];

  const $ = (id) => document.getElementById(id);
  const nf = new Intl.NumberFormat("en");

  // sessionStorage holds the sign-in; localStorage only the viewer's
  // conveniences. Either can be missing or throw (a private window), and the
  // page works without them.
  const session = storage(() => window.sessionStorage);
  const prefs = storage(() => window.localStorage);
  function storage(get) {
    return {
      get(k) { try { return get().getItem(k); } catch { return null; } },
      set(k, v) { try { get().setItem(k, v); } catch { /* kept for this page only */ } },
      del(k) { try { get().removeItem(k); } catch { /* nothing kept */ } },
    };
  }

  const state = {
    token: session.get("observatory.token"),
    refresh: session.get("observatory.refresh"),
    hours: RANGES.includes(Number(prefs.get("observatory.hours"))) ? Number(prefs.get("observatory.hours")) : 1,
    tab: "overview",
    // Samples by `at_ms`, merged from the series (each read asks only for the
    // fields its charts need) and the stream (whole)
    samples: new Map(),
    sorted: null,
    // How far back each set of fields has been read, as Unix ms
    covered: { base: Infinity, load: Infinity },
    annotations: new Map(),
    wire: [],
    wireKind: "all",
    charts: {},
    lastSampleAt: null,
    abort: null,
    live: "off",
  };

  // ---------------------------------------------------------------------------
  // Words

  const plural = (n, one, many) => `${nf.format(n)} ${n === 1 ? one : many}`;
  const ms = (v) => (v == null ? "–" : `${Number(v < 10 ? v.toFixed(2) : v.toFixed(1))} ms`);
  const clock = (t) => new Date(t).toLocaleTimeString("en-GB", { hour: "2-digit", minute: "2-digit" });
  const clockSecs = (t) => new Date(t).toLocaleTimeString("en-GB");
  const day = (t) => new Date(t).toLocaleDateString("en-GB", { day: "numeric", month: "short", year: "numeric" });
  const item = (id) => String(id).replace(/_/g, " ");

  function ago(t) {
    const s = Math.max(0, Math.round((Date.now() - t) / 1000));
    if (s < 60) return `${s} s ago`;
    const m = Math.round(s / 60);
    if (m < 60) return `${m} min ago`;
    const h = Math.floor(m / 60);
    if (h < 48) return `${h} h ${m % 60} min ago`;
    return `${Math.floor(h / 24)} days ago`;
  }

  const window_ = () => ({ 1: "the last hour", 6: "the last six hours", 24: "the last day" }[state.hours]);
  /** "over the last hour", or "since 14:05" when the samples don't reach that far */
  function since(first) {
    if (first.at_ms <= rangeFrom() + 2 * SAMPLE_MS) return `over ${window_()}`;
    return `since ${clock(first.at_ms - first.since_ms)}`;
  }

  /** A knob's value as `Unit::format` says it */
  function unit(u, v) {
    const trim = (x) => String(Number(x.toFixed(4)));
    switch (u) {
      case "fraction": return `${trim(v * 100)}%`;
      case "percent": return `${trim(v)}%`;
      case "strands": return `${trim(v)} Strands`;
      case "multiplier": return `×${trim(v)}`;
      default: return trim(v);
    }
  }

  // ---------------------------------------------------------------------------
  // Signing in

  async function login(email, password) {
    let res;
    try {
      res = await fetch("/auth/login", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ email, password }),
      });
    } catch {
      throw new Error("The server didn't answer. Is it running?");
    }
    if (res.status === 401) throw new Error("That email and password don't match an account here.");
    if (res.status === 429) throw new Error("Too many tries from here. Wait a minute and try again.");
    if (!res.ok) throw new Error(`The server couldn't sign you in (${res.status}).`);
    const auth = await res.json();
    if (!auth.account?.is_admin) {
      endSession(auth.token, auth.refresh_token);
      throw new Error("That account isn't an admin's. The Observatory is kept for the city's admins: sign in with one.");
    }
    keep(auth);
  }

  function keep(auth) {
    state.token = auth.token;
    state.refresh = auth.refresh_token ?? null;
    session.set("observatory.token", state.token);
    if (state.refresh) session.set("observatory.refresh", state.refresh);
    else session.del("observatory.refresh");
  }

  function endSession(token, refresh_token) {
    fetch("/auth/logout", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ token, refresh_token }),
    }).catch(() => {});
  }

  // One refresh at a time: the stream and a series read may both find the
  // token expired at once, and a refresh token is good for one use
  let refreshing = null;
  function refresh() {
    if (!state.refresh) return Promise.resolve(false);
    refreshing ??= fetch("/auth/refresh", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ refresh_token: state.refresh }),
    })
      .then(async (res) => {
        if (!res.ok) return false;
        keep(await res.json());
        return true;
      })
      .catch(() => false)
      .finally(() => { refreshing = null; });
    return refreshing;
  }

  class SignedOut extends Error {}

  /** An admin route, refreshing the token once if it has run out */
  async function api(path, init = {}) {
    for (let attempt = 0; ; attempt++) {
      const res = await fetch(path, {
        ...init,
        headers: { ...(init.headers ?? {}), Authorization: `Bearer ${state.token}` },
      });
      if (res.status === 401 && attempt === 0 && (await refresh())) continue;
      if (res.status === 401) {
        signOut("Your session ran out. Sign in again.");
        throw new SignedOut();
      }
      if (res.status === 403) {
        signOut("That account isn't an admin's any more.");
        throw new SignedOut();
      }
      return res;
    }
  }

  async function getJson(path) {
    const res = await api(path);
    if (!res.ok) throw new Error(`${path}: ${res.status}`);
    return res.json();
  }

  function signOut(why) {
    if (state.token) endSession(state.token, state.refresh);
    state.token = state.refresh = null;
    session.del("observatory.token");
    session.del("observatory.refresh");
    state.abort?.abort();
    state.abort = null;
    for (const chart of Object.values(state.charts)) chart.destroy();
    state.charts = {};
    state.samples.clear();
    state.sorted = null;
    state.covered = { base: Infinity, load: Infinity };
    state.annotations.clear();
    state.wire = [];
    setLive("off");
    showLogin(why);
  }

  // ---------------------------------------------------------------------------
  // The samples

  const rangeFrom = () => Date.now() - state.hours * 3_600_000;

  function samples() {
    state.sorted ??= [...state.samples.values()].sort((a, b) => a.at_ms - b.at_ms);
    return state.sorted;
  }
  function inRange(has = () => true) {
    const from = rangeFrom();
    return samples().filter((s) => s.at_ms > from && has(s));
  }
  const latest = (has = () => true) => {
    const all = samples();
    for (let i = all.length - 1; i >= 0; i--) if (has(all[i])) return all[i];
    return null;
  };

  function addSample(sample) {
    const had = state.samples.get(sample.at_ms);
    state.samples.set(sample.at_ms, had ? Object.assign(had, sample) : sample);
    state.sorted = null;
    // Keep a day, the ring's length
    const old = Date.now() - DAY_MS - SAMPLE_MS;
    if (state.samples.size > 9000) {
      for (const at of state.samples.keys()) if (at < old) state.samples.delete(at);
    }
  }

  function addAnnotation(a) {
    if (state.annotations.has(a.id)) return false;
    state.annotations.set(a.id, a);
    addWire({ kind: "annotation", at_ms: a.at_ms, annotation: a });
    return true;
  }

  // The fields each set of charts reads: the tick's `load` is most of a
  // sample, so it's read only for the performance tab (#411)
  const FIELDS = { base: "players,strands", load: "load" };

  async function readSeries(set) {
    const from = rangeFrom();
    if (state.covered[set] <= from) return;
    const to = Math.min(state.covered[set], Date.now());
    const series = await getJson(`/admin/series?from_ms=${from}&to_ms=${to}&fields=${FIELDS[set]}`);
    for (const s of series.samples) addSample(s);
    for (const a of series.annotations) addAnnotation(a);
    state.covered[set] = from;
  }

  // ---------------------------------------------------------------------------
  // The stream

  async function stream() {
    let wait = 1000;
    while (state.token) {
      const abort = new AbortController();
      state.abort = abort;
      try {
        const res = await api("/admin/stream", { signal: abort.signal });
        if (!res.ok || !res.body) throw new Error(`stream: ${res.status}`);
        // After a gap, the samples missed are in the series
        if (state.lastSampleAt) {
          state.covered = { base: Infinity, load: Infinity };
          await Promise.all([readSeries("base"), state.tab === "performance" ? readSeries("load") : null]);
        }
        setLive("live");
        wait = 1000;
        const reader = res.body.pipeThrough(new TextDecoderStream()).getReader();
        let buffer = "";
        for (;;) {
          const { value, done } = await reader.read();
          if (done) break;
          buffer += value.replace(/\r\n/g, "\n");
          let end;
          while ((end = buffer.indexOf("\n\n")) >= 0) {
            const block = buffer.slice(0, end);
            buffer = buffer.slice(end + 2);
            const data = block.split("\n").filter((l) => l.startsWith("data:")).map((l) => l.slice(5).trimStart());
            if (data.length) receive(JSON.parse(data.join("\n")));
          }
        }
      } catch (e) {
        if (e instanceof SignedOut || abort.signal.aborted) return;
        console.warn("[observatory] stream", e);
      }
      if (!state.token) return;
      setLive("waiting");
      await new Promise((r) => setTimeout(r, wait));
      wait = Math.min(wait * 2, 15_000);
    }
  }

  function receive(feed) {
    switch (feed.type) {
      case "sample": {
        const { type, ...sample } = feed;
        addSample(sample);
        state.lastSampleAt = Date.now();
        tickLive();
        render(["overview", "economy", "performance", "town"]);
        break;
      }
      case "annotation": {
        const { type, ...a } = feed;
        if (addAnnotation(a)) {
          for (const chart of Object.values(state.charts)) chart.redraw();
          if (a.kind === "tunable_changed") loadKnobs();
          render(["events"]);
        }
        break;
      }
      // A Gazette's annotation follows it, and that's what the wire shows
      case "gazette": break;
      default:
        addWire({ kind: feed.type, at_ms: feed.type === "fill" ? Date.parse(feed.trade.at) || Date.now() : Date.now(), feed });
        render(["events"]);
    }
  }

  function setLive(live) {
    state.live = live;
    $("live").dataset.state = live;
    tickLive();
  }
  function tickLive() {
    const text = {
      off: "Not yet live",
      waiting: "Reconnecting…",
      live: state.lastSampleAt ? `Live · sampled ${ago(state.lastSampleAt)}` : "Live · waiting for a sample",
    }[state.live];
    $("live-text").textContent = text;
  }

  // ---------------------------------------------------------------------------
  // Charts: uPlot, coloured from the page's tokens, with the annotations as
  // marks and a line under each naming the one nearest the cursor

  const token = (name) => getComputedStyle(document.documentElement).getPropertyValue(name).trim();

  function lineChart(el, spec) {
    const note = document.createElement("p");
    note.className = "note";
    el.after(note);
    let u = null;
    let shown = [];

    function build() {
      const font = '12px "Source Sans 3", system-ui, sans-serif';
      const axis = {
        stroke: token("--axis"),
        font,
        grid: { stroke: token("--grid"), width: 1 },
        ticks: { stroke: token("--grid"), width: 1, size: 4 },
      };
      const opts = {
        width: Math.max(el.clientWidth, 280),
        height: spec.height ?? 260,
        cursor: { points: { size: 6 }, drag: { x: false, y: false } },
        scales: {
          x: { time: true, range: () => [rangeFrom() / 1000, Date.now() / 1000] },
          // A log scale for the tick, so a mean of half a millisecond still
          // shows under its hundred
          y: spec.log
            ? { distr: 3, log: 10 }
            : { range: (u, min, max) => [Math.min(0, min ?? 0), (max ?? 1) * 1.1 || 1] },
        },
        axes: [
          { ...axis, values: (u, vals) => vals.map((v) => clock(v * 1000)) },
          {
            ...axis,
            size: 56,
            values: (u, vals) => vals.map((v) => (v == null ? "" : spec.axis(v))),
            ...(spec.log ? decades(axis) : {}),
          },
        ],
        series: [
          { value: (u, v) => (v == null ? "–" : clock(v * 1000)) },
          ...spec.series.map((s) => ({
            label: s.label,
            stroke: token(s.color),
            width: s.width ?? 1.75,
            dash: s.dash,
            points: { show: false },
            spanGaps: false,
            value: (u, v) => (v == null ? "–" : spec.legend(v)),
          })),
        ],
        plugins: [marks(), latestInLegend()],
      };
      u = new uPlot(opts, data(), el);
    }

    // On a log scale, a label and a rule at each power of ten and none between
    function decades(axis) {
      const filter = (u, splits) => splits.map((v) => (Math.abs(Math.log10(v) % 1) < 1e-9 ? v : null));
      return { filter, grid: { ...axis.grid, filter }, ticks: { ...axis.ticks, filter } };
    }

    function data() {
      shown = inRange(spec.has);
      // A log scale has no zero: a phase that didn't run is a gap
      const value = spec.log ? (f) => (s) => { const v = f(s); return v > 0 ? v : null; } : (f) => f;
      return [shown.map((s) => s.at_ms / 1000), ...spec.series.map((s) => shown.map(value(s.value)))];
    }

    // With the cursor off the chart, the legend gives the newest values
    // rather than dashes
    function latestInLegend() {
      const settle = (u) => {
        if (u.cursor.idx == null && u.data[0].length) u.setLegend({ idx: u.data[0].length - 1 });
      };
      return { hooks: { ready: [settle], setData: [settle], setCursor: [settle] } };
    }

    function marks() {
      const inView = () => {
        const from = rangeFrom();
        return [...state.annotations.values()].filter((a) => a.at_ms > from).sort((a, b) => a.at_ms - b.at_ms);
      };
      const say = (a, near) => {
        note.innerHTML = "";
        if (!a) return;
        const b = document.createElement("b");
        b.textContent = `◆ ${clock(a.at_ms)}`;
        note.append(b, near ? a.text : `Latest mark: ${a.text}`);
      };
      return {
        hooks: {
          draw: [(u) => {
            const { ctx } = u;
            const { top, height, left, width } = u.bbox;
            const dpr = devicePixelRatio || 1;
            ctx.save();
            ctx.strokeStyle = ctx.fillStyle = token("--marker");
            ctx.lineWidth = dpr;
            ctx.setLineDash([3 * dpr, 4 * dpr]);
            const list = inView();
            for (const a of list) {
              const x = Math.round(u.valToPos(a.at_ms / 1000, "x", true)) + 0.5;
              if (x < left || x > left + width) continue;
              ctx.beginPath();
              ctx.moveTo(x, top);
              ctx.lineTo(x, top + height);
              ctx.stroke();
              const r = 4 * dpr;
              ctx.beginPath();
              ctx.moveTo(x, top - r);
              ctx.lineTo(x + r, top);
              ctx.lineTo(x, top + r);
              ctx.lineTo(x - r, top);
              ctx.closePath();
              ctx.fill();
            }
            ctx.restore();
            if (u.cursor.left == null || u.cursor.left < 0) say(list.at(-1), false);
          }],
          setCursor: [(u) => {
            const list = inView();
            if (u.cursor.left == null || u.cursor.left < 0) return say(list.at(-1), false);
            let best = null;
            let bestD = 8;
            for (const a of list) {
              const d = Math.abs(u.valToPos(a.at_ms / 1000, "x") - u.cursor.left);
              if (d < bestD) { best = a; bestD = d; }
            }
            if (best) say(best, true);
            else say(list.at(-1), false);
          }],
        },
      };
    }

    const resize = new ResizeObserver(() => {
      if (u && el.clientWidth > 0) u.setSize({ width: el.clientWidth, height: spec.height ?? 260 });
    });
    resize.observe(el);

    return {
      update() {
        if (!u) build();
        else u.setData(data());
      },
      redraw() { u?.redraw(false, true); },
      rebuild() { u?.destroy(); u = null; note.innerHTML = ""; build(); },
      destroy() { resize.disconnect(); u?.destroy(); u = null; note.remove(); },
      shown: () => shown,
    };
  }

  const count = (v) => nf.format(Math.round(v));
  const short = (v) => (Math.abs(v) >= 1e6 ? `${(v / 1e6).toFixed(1)}M` : Math.abs(v) >= 1e4 ? `${Math.round(v / 1e3)}k` : nf.format(v));

  const CHARTS = {
    players: () => lineChart($("chart-players"), {
      has: (s) => s.players,
      axis: (v) => count(v),
      legend: count,
      series: [
        { label: "Citizens", color: "--s1", value: (s) => s.players.citizens },
        { label: "People", color: "--s2", value: (s) => s.players.people },
      ],
    }),
    strands: () => lineChart($("chart-strands"), {
      has: (s) => s.strands,
      axis: short,
      legend: (v) => `${count(v)} Strands`,
      series: [
        { label: "Citizens", color: "--s1", value: (s) => s.strands.citizens },
        { label: "People", color: "--s2", value: (s) => s.strands.people },
        { label: "Admins", color: "--s3", value: (s) => s.strands.admins },
        { label: "Escrow", color: "--s4", value: (s) => s.strands.escrow },
      ],
    }),
    tick: () => lineChart($("chart-tick"), {
      has: (s) => s.load,
      log: true,
      axis: (v) => (v == null ? "" : `${v} ms`),
      legend: ms,
      series: [
        { label: "Mean", color: "--s1", value: (s) => s.load.tick_mean_ms },
        { label: "p95", color: "--s2", value: (s) => s.load.tick_p95_ms },
        { label: "Budget", color: "--budget", width: 1, dash: [5, 4], value: (s) => s.load.tick_period_ms },
      ],
    }),
  };
  const PAGE_CHARTS = { overview: ["players"], economy: ["strands"], performance: ["tick"] };

  function chart(name) {
    state.charts[name] ??= CHARTS[name]();
    state.charts[name].update();
  }

  // ---------------------------------------------------------------------------
  // The pages: each story's sentence, from what the charts show

  function overview() {
    chart("players");
    const s = latest((s) => s.players);
    if (!s) {
      $("players-headline").textContent = state.live === "live" ? "No samples yet: the sampler takes one every ten seconds." : "Waiting for the first sample.";
      $("overview-dek").textContent = "";
      return;
    }
    const { citizens, people } = s.players;
    $("players-headline").textContent =
      citizens + people === 0 ? "Nobody is about."
        : citizens === 0 ? `${plural(people, "person is", "people are")} about, and no citizens.`
          : people === 0 ? `${plural(citizens, "citizen is", "citizens are")} about, and nobody else.`
            : `${plural(citizens, "citizen", "citizens")} and ${plural(people, "person", "people")} are about.`;
    const peak = Math.max(citizens + people, ...inRange((s) => s.players).map((s) => s.players.citizens + s.players.people));
    $("overview-dek").textContent =
      `At most ${nf.format(peak)} at once in ${window_()}. The server started ${ago(s.booted_ms)}, on ${day(s.booted_ms)} at ${clock(s.booted_ms)}.`;
  }

  const CLASSES = {
    citizens: (p) => `The citizens hold ${p}`,
    people: (p) => `People hold ${p}`,
    admins: (p) => `Admins hold ${p}`,
    escrow: (p) => `Escrow holds ${p}`,
  };

  function economy() {
    chart("strands");
    const s = latest((s) => s.strands);
    if (!s) return void ($("strands-headline").textContent = "Waiting for the first sample.");
    const total = Object.values(s.strands).reduce((a, b) => a + b, 0);
    if (total === 0) return void ($("strands-headline").textContent = "Nobody holds a Strand yet.");
    const [top, held] = Object.entries(s.strands).sort((a, b) => b[1] - a[1])[0];
    const share = `${Math.round((100 * held) / total)}%`;
    let line = `${CLASSES[top](share)} of the city's ${nf.format(total)} Strands.`;
    const first = inRange((s) => s.strands)[0];
    if (first && first !== s) {
      const was = Object.values(first.strands).reduce((a, b) => a + b, 0);
      const d = total - was;
      line += d === 0 ? ` Unchanged ${since(first)}.` : ` ${d > 0 ? "Up" : "Down"} ${nf.format(Math.abs(d))} ${since(first)}.`;
    }
    $("strands-headline").textContent = line;
  }

  async function performance() {
    if (state.covered.load > rangeFrom()) {
      $("tick-headline").textContent = "Reading the tick's history.";
      try { await readSeries("load"); } catch (e) { if (e instanceof SignedOut) return; console.warn(e); }
    }
    chart("tick");
    const s = latest((s) => s.load);
    if (!s) return void ($("tick-headline").textContent = "Waiting for the first sample.");
    const l = s.load;
    let line = l.ticks === 0
      ? "No tick ran in the last ten seconds."
      : `The tick takes ${ms(l.tick_mean_ms)} on average and ${ms(l.tick_p95_ms)} at p95, of its ${ms(l.tick_period_ms)}.`;
    const over = inRange((s) => s.load).reduce((n, s) => n + (s.load.ticks_over ?? 0), 0);
    line += over === 0 ? ` None ran over in ${window_()}.` : ` ${plural(over, "tick", "ticks")} ran over in ${window_()}.`;
    $("tick-headline").textContent = line;
  }

  let roster = null;
  async function town(reload = false) {
    if (!roster || reload) {
      try { roster = await getJson("/admin/citizens"); } catch (e) {
        if (!(e instanceof SignedOut)) $("town-headline").textContent = "The roster couldn't be read.";
        return;
      }
      const body = $("roster").tBodies[0];
      body.replaceChildren(...(roster.length ? roster.map(citizenRow) : [emptyRow(4, "No citizens yet. scripts/town.sh up makes a town of them.")]));
    }
    const on = latest((s) => s.players)?.players.citizens;
    $("town-headline").textContent = roster.length === 0
      ? "This server has made no citizens."
      : on == null ? `The town has ${plural(roster.length, "citizen", "citizens")}.`
          : on === 0 ? `The town has ${plural(roster.length, "citizen", "citizens")}, and none of them is about now.`
            : `The town has ${plural(roster.length, "citizen", "citizens")}, and ${nf.format(on)} ${on === 1 ? "is" : "are"} about now.`;
  }

  function citizenRow(c) {
    const tr = document.createElement("tr");
    const key = c.key
      ? [text("code", `${c.key.prefix}…`), ` ${c.key.scopes.join(" + ")}`]
      : ["none, so it can't sign in"];
    tr.append(
      cell(["◇ ", c.name]),
      cell(day(c.created_at), "dim"),
      cell(key, c.key ? "" : "dim"),
      cell(c.key?.last_used_at ? ago(Date.parse(c.key.last_used_at)) : "never", "dim"),
    );
    return tr;
  }

  async function loadKnobs() {
    let knobs;
    try { ({ knobs } = await getJson("/admin/tunables")); } catch (e) {
      if (!(e instanceof SignedOut)) $("tuning-headline").textContent = "The knobs couldn't be read.";
      return;
    }
    const off = knobs.filter((k) => k.value !== k.default);
    $("tuning-headline").textContent = off.length === 0
      ? `Every one of the ${knobs.length} knobs is at its default.`
      : off.length <= 3
        ? `${off.length} of ${knobs.length} knobs ${off.length === 1 ? "is" : "are"} off ${off.length === 1 ? "its default" : "their defaults"}: ${off.map((k) => `${k.label} at ${unit(k.unit, k.value)}`).join(", ")}.`
        : `${off.length} of ${knobs.length} knobs are off their defaults.`;
    $("knobs").tBodies[0].replaceChildren(...knobs.map((k) => {
      const tr = document.createElement("tr");
      if (k.value !== k.default) tr.className = "off";
      tr.append(
        cell([k.label, " ", text("code", k.key)]),
        cell(unit(k.unit, k.value), "num now"),
        cell(unit(k.unit, k.default), "num dim"),
        cell(k.unit === "strands" ? `${nf.format(k.min)} to ${nf.format(k.max)} Strands` : `${unit(k.unit, k.min)} to ${unit(k.unit, k.max)}`, "num dim"),
        cell(k.chart.charAt(0).toUpperCase() + k.chart.slice(1), "dim"),
      );
      return tr;
    }));
  }

  // ---------------------------------------------------------------------------
  // The wire

  const NOTE_TAGS = {
    boot: "Boot", terrain_committed: "Terrain", gazette_published: "Gazette", tunable_changed: "Knob",
    town_up: "Town", town_down: "Town", watch_fired: "Watch",
  };

  /** What an entry is filed under, its tag, and its sentence */
  function entry(e) {
    if (e.kind === "annotation") {
      const a = e.annotation;
      return { group: a.kind === "gazette_published" ? "gazette" : "annotation", tag: NOTE_TAGS[a.kind] ?? "Note", text: a.text };
    }
    const f = e.feed;
    switch (f.type) {
      case "fill": {
        const t = f.trade;
        return { group: "fill", tag: "Fill", text: `${nf.format(t.quantity)} ${item(t.item_type)}, grade ${t.grade}, at ${nf.format(t.price)} Strands each.` };
      }
      case "deed":
        return { group: "deed", tag: "Deed", text: `Lot ${f.terrain_id} ${f.change}.` };
      case "warning":
        return { group: "warning", tag: "Warning", text: f.text };
      case "lagged":
        return { group: "warning", tag: "Behind", text: `This page fell behind the stream, and ${plural(f.missed, "thing", "things")} went past it.` };
      default:
        return { group: "annotation", tag: f.type, text: JSON.stringify(f) };
    }
  }

  function addWire(e) {
    state.wire.push({ ...e, ...entry(e) });
    state.wire.sort((a, b) => a.at_ms - b.at_ms);
    if (state.wire.length > 600) state.wire.splice(0, state.wire.length - 600);
  }

  function events() {
    const from = rangeFrom();
    const recent = state.wire.filter((e) => e.at_ms > from);
    const warnings = recent.filter((e) => e.group === "warning").length;
    $("events-headline").textContent = recent.length === 0
      ? `Nothing on the wire in ${window_()}.`
      : `${plural(recent.length, "thing", "things")} on the wire in ${window_()}${warnings ? `, ${nf.format(warnings)} of them ${warnings === 1 ? "a warning" : "warnings"}` : ""}.`;
    const shown = state.wire.filter((e) => state.wireKind === "all" || e.group === state.wireKind).slice(-200).reverse();
    $("wire").replaceChildren(...(shown.length ? shown.map((e) => {
      const li = document.createElement("li");
      const time = text("time", clockSecs(e.at_ms));
      time.dateTime = new Date(e.at_ms).toISOString();
      const tag = text("span", e.tag);
      tag.className = "kind";
      tag.dataset.kind = e.group;
      const words = text("span", e.text);
      words.className = "text";
      li.append(time, tag, words);
      return li;
    }) : [Object.assign(document.createElement("li"), { className: "empty", textContent: "Nothing yet. The wire fills as things happen." })]));
  }

  // ---------------------------------------------------------------------------
  // Small DOM helpers

  function text(tag, s) {
    const el = document.createElement(tag);
    el.textContent = s;
    return el;
  }
  function cell(content, cls = "") {
    const td = document.createElement("td");
    if (cls) td.className = cls;
    td.append(...[content].flat());
    return td;
  }
  function emptyRow(span, s) {
    const tr = document.createElement("tr");
    tr.className = "empty";
    const td = cell(s);
    td.colSpan = span;
    tr.append(td);
    return tr;
  }

  // ---------------------------------------------------------------------------
  // The tabs, the range and the theme

  const PAGES = { overview, economy, performance, town: () => town(), tuning: loadKnobs, events };

  /** Redraw the open page if it's one of `pages` */
  function render(pages) {
    if (pages.includes(state.tab)) {
      Promise.resolve(PAGES[state.tab]()).catch((e) => { if (!(e instanceof SignedOut)) console.warn(e); });
    }
  }

  function show(tab) {
    if (!PAGES[tab]) tab = "overview";
    state.tab = tab;
    for (const a of document.querySelectorAll("#tabs a")) a.setAttribute("aria-selected", String(a.dataset.tab === tab));
    for (const p of document.querySelectorAll(".page")) p.hidden = p.dataset.page !== tab;
    // A chart built while hidden had no width; size it now it shows
    for (const name of PAGE_CHARTS[tab] ?? []) state.charts[name]?.rebuild();
    render([tab]);
  }

  function setRange(hours) {
    state.hours = hours;
    prefs.set("observatory.hours", String(hours));
    for (const b of document.querySelectorAll("#range button")) b.setAttribute("aria-pressed", String(Number(b.dataset.hours) === hours));
    if (!state.token) return;
    readSeries("base").then(() => render([state.tab])).catch((e) => { if (!(e instanceof SignedOut)) console.warn(e); });
    render([state.tab]);
  }

  function setTheme(theme) {
    if (theme) document.documentElement.dataset.theme = theme;
    else delete document.documentElement.dataset.theme;
    for (const chart of Object.values(state.charts)) chart.rebuild();
  }
  function currentTheme() {
    return document.documentElement.dataset.theme
      ?? (matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light");
  }

  // ---------------------------------------------------------------------------
  // Start

  function showLogin(why) {
    document.body.classList.remove("signed-in");
    $("console-view").hidden = true;
    $("login-view").hidden = false;
    const error = $("login-error");
    error.hidden = !why;
    error.textContent = why ?? "";
    $("login-email").focus();
  }

  async function showConsole() {
    document.body.classList.add("signed-in");
    $("login-view").hidden = true;
    $("console-view").hidden = false;
    show(location.hash.slice(1));
    try {
      await readSeries("base");
    } catch (e) {
      if (e instanceof SignedOut) return;
      console.warn(e);
    }
    render([state.tab]);
    stream();
  }

  function start() {
    $("dateline-day").textContent = new Date().toLocaleDateString("en-GB", { weekday: "long", day: "numeric", month: "long", year: "numeric" });
    $("dateline-server").textContent = location.host;

    const saved = prefs.get("observatory.theme");
    if (saved === "light" || saved === "dark") setTheme(saved);
    $("theme").addEventListener("click", () => {
      const next = currentTheme() === "dark" ? "light" : "dark";
      prefs.set("observatory.theme", next);
      setTheme(next);
    });
    matchMedia("(prefers-color-scheme: dark)").addEventListener("change", () => {
      if (!document.documentElement.dataset.theme) setTheme(null);
    });

    for (const b of document.querySelectorAll("#range button")) b.addEventListener("click", () => setRange(Number(b.dataset.hours)));
    setRange(state.hours);
    for (const b of document.querySelectorAll("#filters button")) {
      b.addEventListener("click", () => {
        state.wireKind = b.dataset.kind;
        for (const o of document.querySelectorAll("#filters button")) o.setAttribute("aria-pressed", String(o === b));
        events();
      });
    }
    addEventListener("hashchange", () => show(location.hash.slice(1)));
    $("sign-out").addEventListener("click", () => signOut(null));

    $("login-form").addEventListener("submit", async (ev) => {
      ev.preventDefault();
      const submit = $("login-submit");
      submit.disabled = true;
      submit.textContent = "Signing in…";
      try {
        await login($("login-email").value.trim(), $("login-password").value);
        $("login-password").value = "";
        await showConsole();
      } catch (e) {
        showLogin(e.message);
      } finally {
        submit.disabled = false;
        submit.textContent = "Sign in";
      }
    });

    setInterval(tickLive, 1000);
    if (state.token) showConsole();
    else showLogin(null);
  }

  start();
})();
