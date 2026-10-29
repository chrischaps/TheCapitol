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
// per tab that later packages grow (D2 the economy, D3.1 performance, D4.2
// the town, D6.2 tuning). D1.2 made the overview: tiles for health, the tick
// and who's on, the books' drift, and the newest on the wire. D1.3 adds
// the map. D2.1 grows the economy tab: where the Strands come from and
// go, what buy orders hold, and how fast the money moves. D2.3 adds prices
// against target, and D2.4 what's made and used, the stock, and the
// workshops. D4.2 grows the town tab: who's on, the roster from the
// town's report, how the questions were settled and what that costs.
// D6.2 makes the tuning tab: each knob beside its chart, set and reset
// live, and presets as TOML.
// D4.3 adds a page for one citizen: their Strands, their decisions,
// their dot.

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
    covered: { base: Infinity, load: Infinity, money: Infinity, making: Infinity, town: Infinity },
    annotations: new Map(),
    wire: [],
    wireKind: "all",
    charts: {},
    lastSampleAt: null,
    abort: null,
    live: "off",
    // The last answer from /health, for the overview's health tile
    health: null,
    // The Gini watch's threshold, from the knobs, drawn over the Gini
    giniWatch: null,
    // Each item's target price from /catalog, once read (D2.3)
    targets: null,
  };

  // ---------------------------------------------------------------------------
  // Words

  const plural = (n, one, many) => `${nf.format(n)} ${n === 1 ? one : many}`;
  const ms = (v) => (v == null ? "–" : `${Number(v < 10 ? v.toFixed(2) : v.toFixed(1))} ms`);
  const clock = (t) => new Date(t).toLocaleTimeString("en-GB", { hour: "2-digit", minute: "2-digit" });
  const clockSecs = (t) => new Date(t).toLocaleTimeString("en-GB");
  const day = (t) => new Date(t).toLocaleDateString("en-GB", { day: "numeric", month: "short", year: "numeric" });
  const item = (id) => String(id).replace(/_/g, " ");
  const capital = (s) => s.charAt(0).toUpperCase() + s.slice(1);

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
    state.covered = { base: Infinity, load: Infinity, money: Infinity, making: Infinity, town: Infinity };
    state.annotations.clear();
    state.wire = [];
    mapStop();
    map.walkers.clear();
    map.layers = map.nodes = null;
    map.follow = map.hover = null;
    Object.assign(townView, { latest: undefined, readAt: 0, made: null });
    Object.assign(citizenView, { detail: undefined, readAt: 0 });
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
    state.samples.set(sample.at_ms, had ? merge(had, sample) : sample);
    state.sorted = null;
    // Keep a day, the ring's length
    const old = Date.now() - DAY_MS - SAMPLE_MS;
    if (state.samples.size > 9000) {
      for (const at of state.samples.keys()) if (at < old) state.samples.delete(at);
    }
  }

  /** `part` put into `into`, object by object: a series asks for paths
   * inside a sample (#411), so one set's `load` holds a few of its numbers,
   * and mustn't take the place of those another set or the stream brought */
  function merge(into, part) {
    for (const [k, v] of Object.entries(part)) {
      const had = into[k];
      const objects = had && v && typeof had === "object" && typeof v === "object" && !Array.isArray(had) && !Array.isArray(v);
      if (objects) merge(had, v);
      else into[k] = v;
    }
    return into;
  }

  function addAnnotation(a) {
    if (state.annotations.has(a.id)) return false;
    state.annotations.set(a.id, a);
    addWire({ kind: "annotation", at_ms: a.at_ms, annotation: a });
    return true;
  }

  // The fields each set of charts reads, as paths into a sample (#411). The
  // base has the few numbers of the tick the overview's tiles and the tick's
  // chart draw. The rest of `load` that's drawn, the queues, sockets and
  // pool, and four numbers of each phase and loop for their table (about
  // 17 MB a day, #572), is read only for the performance tab, with the event bus's backlog beside it
  // (D3.1); the money's flows and trades, and the wealth by trade, only for
  // the economy tab, and the units and crafts too (D2.4)
  const TICK_FIELDS = ["ticks", "secs", "tick_mean_ms", "tick_p95_ms", "tick_p99_ms", "tick_period_ms", "ticks_over", "sockets_open"];
  const LOAD_FIELDS = [
    "commands", "command_queue_p99", "writer_queued", "writer_lag_ms", "events_lagged", "socket_resyncs",
    "sockets_opened", "sockets_dropped", "players_in_world", "db_pool_size", "db_pool_idle", "db_transactions_per_sec",
  ];
  const WORK_FIELDS = ["runs", "secs", "p95_ms", "slow_statements"];
  const FIELDS = {
    base: ["players", "strands", "drift", ...TICK_FIELDS.map((f) => `load.${f}`)].join(","),
    load: [
      ...LOAD_FIELDS.map((f) => `load.${f}`),
      ...["phases", "loops"].flatMap((part) => WORK_FIELDS.map((f) => `load.${part}.*.${f}`)),
      "events",
    ].join(","),
    money: "strand_flows,new_accounts,grants,market,bazaar,distribution,books",
    making: "units,unit_flows,crafts",
    town: "town",
  };

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
          state.covered = { base: Infinity, load: Infinity, money: Infinity, making: Infinity, town: Infinity };
          await Promise.all([
            readSeries("base"),
            state.tab === "performance" ? readSeries("load") : null,
            state.tab === "economy" ? readSeries("money") : null,
            state.tab === "economy" ? readSeries("making") : null,
            state.tab === "town" ? readSeries("town") : null,
            state.tab === "tuning" ? Promise.all([readSeries("money"), readSeries("making")]) : null,
          ]);
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
        render(["overview", "economy", "performance", "town", "citizen", "tuning"]);
        break;
      }
      case "annotation": {
        const { type, ...a } = feed;
        if (addAnnotation(a)) {
          for (const chart of Object.values(state.charts)) chart.redraw();
          if (a.kind === "tunable_changed") loadKnobs();
          if (a.kind === "town_up" || a.kind === "town_down") { townView.readAt = 0; render(["town", "citizen"]); }
          render(["events", "overview"]);
        }
        break;
      }
      // A Gazette's annotation follows it, and that's what the wire shows
      case "gazette": break;
      default:
        addWire({ kind: feed.type, at_ms: feed.type === "fill" ? Date.parse(feed.trade.at) || Date.now() : Date.now(), feed });
        if (feed.type === "fill" || feed.type === "deed") layersSoon();
        render(["events", "overview"]);
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
    // A spec's series can be a function, read again on each rebuild: the
    // money's chart has a bar for each kind of flow the range holds
    let series = [];
    let rows = null;
    const bars = uPlot.paths.bars({ size: [0.72, 48], align: 0 });
    // A balance holds until the next move changes it
    const steps = uPlot.paths.stepped({ align: 1 });

    function build() {
      series = typeof spec.series === "function" ? spec.series() : spec.series;
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
          // Binned rows run to the end of the bin still filling, so its bar shows
          x: { time: true, range: () => [rangeFrom() / 1000, Math.max(Date.now() / 1000, binEnd())] },
          // A log scale for the tick, so a mean of half a millisecond still
          // shows under its hundred
          y: spec.log
            ? { distr: 3, log: 10 }
            : { range: spec.range ?? ((u, min, max) => [Math.min(0, (min ?? 0) * 1.1), Math.max(0, (max ?? 1) * 1.1) || 1]) },
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
          ...series.map((s, k) => ({
            label: s.label,
            stroke: token(s.color),
            width: s.bars ? 0 : s.width ?? 1.75,
            dash: s.dash,
            ...(s.bars ? { fill: token(s.color), paths: bars } : {}),
            ...(s.step ? { paths: steps } : {}),
            points: { show: false },
            spanGaps: false,
            // A stacked bar is drawn to its running total, but the legend
            // gives its own amount
            value: (u, v, si, i) => {
              const own = rows?.own?.[s.key] && i != null ? rows.own[s.key][i] : v;
              return own == null ? "–" : (s.legend ?? spec.legend)(own);
            },
          })),
        ],
        plugins: [marks(), latestInLegend()],
      };
      u = new uPlot(opts, data(), el);
    }

    /** Where the last bin ends, in Unix seconds; 0 for rows that aren't bins */
    function binEnd() {
      const x = rows?.x;
      return x?.length > 1 && !rows.moments ? x.at(-1) + (x[1] - x[0]) / 2 : 0;
    }

    // On a log scale, a label and a rule at each power of ten and none between
    function decades(axis) {
      const filter = (u, splits) => splits.map((v) => (Math.abs(Math.log10(v) % 1) < 1e-9 ? v : null));
      return { filter, grid: { ...axis.grid, filter }, ticks: { ...axis.ticks, filter } };
    }

    function data() {
      // Rows a spec works out itself, such as bins: `x` in Unix seconds,
      // `values` drawn and `own` the legend's, each by series key. Rows
      // marked `moments` are points in time, not bins.
      if (spec.rows) {
        rows = spec.rows();
        shown = rows.shown;
        return [rows.x, ...series.map((s) => rows.values[s.key] ?? rows.x.map(() => null))];
      }
      shown = inRange(spec.has);
      // A log scale has no zero: a phase that didn't run is a gap
      const value = spec.log ? (f) => (s) => { const v = f(s); return v > 0 ? v : null; } : (f) => f;
      return [shown.map((s) => s.at_ms / 1000), ...series.map((s) => shown.map(value(s.value)))];
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
  const percent = (v) => `${Math.round(v * 100)}%`;
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
    // The Gini and the richest tenth's share both run 0 to 1, so they share
    // an axis, with the Gini watch's threshold over them (D2.5)
    spread: (el = $("chart-spread")) => lineChart(el, {
      has: (s) => s.distribution?.gini != null,
      range: () => [0, 1],
      axis: (v) => v.toFixed(1),
      legend: (v) => v.toFixed(2),
      series: [
        { label: "Gini", color: "--s1", value: (s) => s.distribution.gini },
        { label: "Richest tenth's share", color: "--s2", legend: percent, value: (s) => s.distribution.top_tenth },
        { label: "Gini watch", color: "--budget", width: 1, dash: [5, 4], value: () => state.giniWatch },
      ],
    }),
    tick: (el = $("chart-tick")) => lineChart(el, {
      has: (s) => s.load,
      log: true,
      axis: (v) => (v == null ? "" : `${v} ms`),
      legend: ms,
      series: [
        { label: "Mean", color: "--s1", value: (s) => s.load.tick_mean_ms },
        { label: "p95", color: "--s2", value: (s) => s.load.tick_p95_ms },
        { label: "p99", color: "--s3", value: (s) => s.load.tick_p99_ms },
        { label: "Budget", color: "--budget", width: 1, dash: [5, 4], value: (s) => s.load.tick_period_ms },
      ],
    }),
  };
  CHARTS.flows = () => lineChart($("chart-flows"), {
    rows: () => money.rows,
    axis: short,
    legend: (v) => `${v > 0 ? "+" : ""}${count(v)}`,
    // Drawn outermost first, so each bar's inner lines paint over it
    series: () => money.lines.map((l) => ({ key: l.key, label: l.label, color: l.color, bars: true })).reverse(),
  });
  // Each group's share of the Strands over its share of the accounts (#559),
  // for those furthest from even, with even drawn through them
  CHARTS.headcount = () => lineChart($("chart-headcount"), {
    has: (s) => s.distribution?.[wealthView.by],
    height: 200,
    range: (u, min, max) => [0, Math.max(2, (max ?? 1) * 1.1)],
    axis: (v) => `${Number(v.toFixed(1))}×`,
    legend: (v) => `${v.toFixed(2)}×`,
    series: () => [
      ...wealthView.lines.map((key, k) => ({
        label: groupName(key), color: `--s${k + 1}`, value: (s) => against(s.distribution, wealthView.by)[key]?.ratio ?? null,
      })),
      { label: "Even", color: "--budget", width: 1, dash: [5, 4], value: () => 1 },
    ],
  });
  CHARTS.escrow = () => lineChart($("chart-escrow"), {
    has: (s) => s.strands,
    height: 160,
    axis: short,
    legend: (v) => `${count(v)} Strands`,
    series: [{ label: "Escrow", color: "--fl-escrow", value: (s) => s.strands.escrow }],
  });
  CHARTS.velocity = () => lineChart($("chart-velocity"), {
    rows: () => money.velocity,
    height: 180,
    axis: (v) => `${Number(v.toFixed(2))}×`,
    legend: (v) => `${v.toFixed(2)}× a game day`,
    series: [{ key: "velocity", label: "Velocity", color: "--s2", width: 2 }],
  });
  CHARTS.making = () => lineChart($("chart-making"), {
    rows: () => making.rows,
    axis: short,
    legend: (v) => `${v > 0 ? "+" : ""}${count(v)}`,
    series: () => making.lines.map((l) => ({ key: l.key, label: l.label, color: l.color, bars: true })).reverse(),
  });
  // The tick's work (D3.1): each phase's mean ms a tick, stacked in bars
  CHARTS.phases = () => lineChart($("chart-phases"), {
    rows: () => perf.rows,
    axis: (v) => `${Number(v.toPrecision(2))} ms`,
    legend: ms,
    // Drawn outermost first, so each bar's inner phases paint over it
    series: () => perf.lines.map((l) => ({ key: l.key, label: l.label, color: l.color, bars: true })).reverse(),
  });
  CHARTS.over = () => lineChart($("chart-over"), {
    rows: () => perf.over,
    height: 160,
    axis: count,
    legend: (v) => plural(v, "tick", "ticks"),
    series: [{ key: "over", label: "Over budget", color: "--down", bars: true }],
  });
  CHARTS.commands = () => lineChart($("chart-commands"), {
    has: (s) => s.load?.ticks > 0 && s.load.command_queue_p99 !== undefined,
    height: 160,
    axis: (v) => Number(v.toPrecision(2)).toString(),
    legend: (v) => `${Number(v.toFixed(2))} commands`,
    series: [
      { label: "A tick", color: "--s1", value: (s) => s.load.commands / s.load.ticks },
      { label: "p99", color: "--s2", value: (s) => s.load.command_queue_p99 },
    ],
  });
  // The writer (#407): saves waiting for it, and how late a tick's were written
  CHARTS.writer = () => lineChart($("chart-writer"), {
    has: (s) => s.load?.writer_lag_ms !== undefined,
    height: 160,
    axis: count,
    legend: (v) => plural(v, "save", "saves"),
    series: [
      { label: "Waiting", color: "--s2", width: 2, value: (s) => s.load.writer_queued },
      { label: "Lag", color: "--s3", legend: ms, value: (s) => s.load.writer_lag_ms },
    ],
  });
  CHARTS.bus = () => lineChart($("chart-bus"), {
    has: (s) => s.load && s.events,
    height: 160,
    axis: count,
    legend: (v) => plural(v, "event", "events"),
    series: [
      { label: "Backlog", color: "--s2", value: (s) => s.events.queued },
      { label: "Missed", color: "--down", value: (s) => missed(s.load) },
      { label: "Resyncs", color: "--s3", legend: (v) => plural(v, "socket", "sockets"), value: (s) => s.load.socket_resyncs },
    ],
  });
  CHARTS.sockets = () => lineChart($("chart-sockets"), {
    has: (s) => s.load,
    height: 160,
    axis: count,
    legend: count,
    series: [
      { label: "Open", color: "--s1", width: 2, value: (s) => s.load.sockets_open },
      { label: "Opened", color: "--s2", value: (s) => s.load.sockets_opened },
      { label: "Dropped", color: "--down", value: (s) => s.load.sockets_dropped },
    ],
  });
  CHARTS.pool = () => lineChart($("chart-pool"), {
    has: (s) => s.load?.db_pool_size > 0,
    height: 160,
    axis: count,
    legend: (v) => plural(v, "connection", "connections"),
    series: [
      { label: "In use", color: "--s2", width: 2, value: (s) => s.load.db_pool_size - s.load.db_pool_idle },
      { label: "Pool", color: "--budget", width: 1, dash: [5, 4], value: (s) => s.load.db_pool_size },
    ],
  });
  CHARTS.txns = () => lineChart($("chart-txns"), {
    has: (s) => s.load,
    height: 160,
    axis: short,
    legend: (v) => `${nf.format(Math.round(v))} a second`,
    series: [{ label: "Transactions", color: "--s1", value: (s) => s.load.db_transactions_per_sec }],
  });
  // The tuning tab's charts (D6.2), each beside its knobs. The sinks and
  // what's made are the economy's stacked bars drawn as a line each, so the
  // line a knob moves reads on its own; the rest are the other tabs' charts
  // drawn again
  CHARTS["tune-sinks"] = () => lineChart($("chart-tune-sinks"), {
    rows: () => sideRows(money, "out", -1, KNOB_SINKS),
    axis: short,
    legend: (v) => `${count(v)} Strands`,
    series: () => sinkLines().map((l) => ({ key: l.key, label: l.label, color: l.color, width: 2 })),
  });
  // The sinks a knob moves have a line even when nothing flowed, so the tax
  // knob always has its tax line beside it, flat at nothing if need be
  const KNOB_SINKS = ["tax", "deeds", "rent", "fees", "withdrawals"];
  function sinkLines() {
    const out = money.lines.filter((l) => l.side === "out");
    const missing = KNOB_SINKS.filter((k) => !out.some((l) => l.key === k))
      .map((key) => ({ key, side: "out", label: LINES[key].label, say: LINES[key].say, color: `--fl-${key}` }));
    return [...out, ...missing];
  }
  CHARTS["tune-production"] = () => lineChart($("chart-tune-production"), {
    rows: () => sideRows(making, "in", 1),
    axis: short,
    legend: (v) => plural(Math.round(v), "unit", "units"),
    series: () => making.lines.filter((l) => l.side === "in").map((l) => ({ key: l.key, label: l.label, color: l.color, width: 2 })),
  });
  CHARTS["tune-distribution"] = () => CHARTS.spread($("chart-tune-distribution"));
  CHARTS["tune-prices"] = () => CHARTS.prices($("chart-tune-prices"));
  CHARTS["tune-tick"] = () => CHARTS.tick($("chart-tune-tick"));

  /**
   * One side of a stacked chart's rows as each line's own amount a bar, `sign`
   * up. Each of `always` the range has no flow of is drawn at nothing where
   * the bin has samples.
   */
  function sideRows(set, side, sign, always = []) {
    const r = set.rows;
    if (!r) return { x: [], shown: [], values: {} };
    const values = {};
    for (const l of set.lines) if (l.side === side) values[l.key] = r.own[l.key].map((v) => (v == null ? null : sign * v || 0));
    const any = Object.values(r.own)[0];
    for (const key of always) values[key] ??= r.x.map((_, i) => (any && any[i] == null ? null : 0));
    return { x: r.x, shown: r.shown, values };
  }

  const PAGE_CHARTS = {
    overview: ["players"],
    economy: ["strands", "flows", "escrow", "velocity", "spread", "prices", "making"],
    performance: ["tick", "phases", "over", "commands", "writer", "bus", "sockets", "pool", "txns"],
    town: ["townOn", "rulings", "cost"],
    tuning: ["tune-sinks", "tune-production", "tune-distribution", "tune-prices", "tune-tick"],
    citizen: ["citizenStrands"],
  };

  function chart(name) {
    state.charts[name] ??= CHARTS[name]();
    state.charts[name].update();
  }

  // ---------------------------------------------------------------------------
  // Prices against target (D2.3): each item's market VWAP over a trailing
  // window, as a share of the price the content balances it to

  // A sample's ten seconds seldom hold a fill, so each point is the VWAP of
  // the fills in the ten minutes up to it
  const TRAIL_MS = 10 * 60_000;
  // The most items the chart draws, the most traded first: one per series colour
  const PRICE_LINES = 5;

  /** Each item's target price, Strands a unit, from the catalog (read once) */
  async function readTargets() {
    if (state.targets) return state.targets;
    const res = await fetch("/catalog");
    if (!res.ok) throw new Error(`/catalog: ${res.status}`);
    const catalog = await res.json();
    state.targets = new Map(catalog.item_types.filter((t) => t.target_price).map((t) => [t.id, t.target_price]));
    return state.targets;
  }

  /** `item`'s trailing VWAP at each of `shown`, null where nothing traded */
  function trailing(shown, item) {
    const out = [];
    let lo = 0;
    let units = 0;
    let strands = 0;
    for (const s of shown) {
      const t = s.market?.[item];
      if (t) { units += t.units; strands += t.strands; }
      while (shown[lo].at_ms <= s.at_ms - TRAIL_MS) {
        const old = shown[lo++].market?.[item];
        if (old) { units -= old.units; strands -= old.strands; }
      }
      out.push(units > 0 ? strands / units : null);
    }
    return out;
  }

  /** The market over the range, by item: fills, units, Strands, and the newest book */
  function marketOver(list) {
    const items = new Map();
    const of = (id) => {
      if (!items.has(id)) items.set(id, { id, trades: 0, units: 0, strands: 0, book: null });
      return items.get(id);
    };
    for (const s of list) {
      for (const [id, t] of Object.entries(s.market ?? {})) {
        const i = of(id);
        i.trades += t.trades;
        i.units += t.units;
        i.strands += t.strands;
      }
    }
    const newest = list.at(-1);
    for (const [id, b] of Object.entries(newest?.books ?? {})) of(id).book = b;
    return items;
  }

  // The chart's items, so it's rebuilt when the most traded change, and each
  // one's trailing price as a share of target by `at_ms`, worked out once a
  // render by `prices`
  let priceLines = [];
  let priceTrail = new Map();
  CHARTS.prices = (el = $("chart-prices")) => lineChart(el, {
    has: (s) => s.market,
    log: true,
    axis: (v) => `${Number((v * 100).toPrecision(2))}%`,
    legend: (v) => `${Math.round(v * 100)}% of target`,
    series: [
      ...priceLines.map((id, k) => ({
        label: capital(item(id)),
        color: `--s${k + 1}`,
        width: 2,
        value: (s) => priceTrail.get(id)?.get(s.at_ms) ?? null,
      })),
      { label: "Target", color: "--marker", width: 1, dash: [5, 4], value: () => 1 },
    ],
  });

  async function prices() {
    let targets;
    try { targets = await readTargets(); } catch (e) {
      console.warn(e);
      return void ($("prices-headline").textContent = "The catalog couldn't be read, so there are no targets to judge by.");
    }
    const list = inRange((s) => s.market);
    const items = marketOver(list);
    const firing = new Set((latest((s) => s.watches)?.watches ?? []).filter((w) => w.kind === "off_target").map((w) => w.item));
    const rows = [...items.values()]
      .filter((i) => targets.has(i.id) && (i.units > 0 || i.book))
      .map((i) => ({ ...i, target: targets.get(i.id), vwap: i.units > 0 ? i.strands / i.units : null, off: firing.has(i.id) }))
      .sort((a, b) => b.strands - a.strands || a.id.localeCompare(b.id));

    // The chart: the most traded, rebuilt when they change
    const lines = rows.filter((r) => r.units > 0).slice(0, PRICE_LINES).map((r) => r.id);
    priceTrail = new Map(lines.map((id) => {
      const target = targets.get(id);
      return [id, new Map(trailing(list, id).map((v, k) => [list[k].at_ms, v == null ? null : v / target]))];
    }));
    if (lines.join() !== priceLines.join()) {
      priceLines = lines;
      for (const name of ["prices", "tune-prices"]) {
        state.charts[name]?.destroy();
        delete state.charts[name];
      }
    }
    chart("prices");

    $("prices").tBodies[0].replaceChildren(...(rows.length ? rows.map(priceRow(list)) : [emptyRow(7, "Nothing with a target has traded or rested on the book in the range.")]));
    $("prices-headline").textContent = pricesHeadline(rows, list);
  }

  function pricesHeadline(rows, list) {
    if (!list.length) return "Waiting for the first sample.";
    const traded = rows.filter((r) => r.vwap != null);
    if (!traded.length) {
      return rows.length ? `Nothing with a target traded ${since(list[0])}, though ${plural(rows.length, "item rests", "items rest")} on the book.` : `Nothing traded on the market ${since(list[0])}.`;
    }
    const off = rows.filter((r) => r.off);
    const far = traded.map((r) => ({ r, by: r.vwap / r.target })).sort((a, b) => Math.abs(Math.log(b.by)) - Math.abs(Math.log(a.by)))[0];
    const at = (r) => `${capital(item(r.id))} trades at ${price(r.vwap)} against a target of ${nf.format(r.target)}`;
    const lead = off.length
      ? `${off.length === 1 ? `${capital(item(off[0].id))} is` : `${plural(off.length, "item is", "items are")}`} off target for a game day.`
      : "";
    if (Math.abs(Math.log(far.by)) < Math.log(1.1)) {
      return `${lead}${lead ? " " : ""}Every item traded ${since(list[0])} is within a tenth of its target.`.trim();
    }
    const pct = Math.round(Math.abs(far.by - 1) * 100);
    return `${lead}${lead ? " Furthest: " : ""}${at(far.r)}, ${pct}% ${far.by < 1 ? "under" : "over"}.`;
  }

  const price = (v) => (v == null ? "–" : v < 10 ? Number(v.toFixed(2)).toString() : nf.format(Math.round(v)));

  function priceRow(list) {
    return (r) => {
      const tr = document.createElement("tr");
      if (r.off) tr.className = "off";
      const by = r.vwap == null ? null : r.vwap / r.target;
      const against = by == null ? "–" : Math.abs(by - 1) < 0.005 ? "on target" : `${by < 1 ? "−" : "+"}${Math.round(Math.abs(by - 1) * 100)}%`;
      const book = r.book
        ? [r.book.bids ? `${nf.format(r.book.bids)} bid at ${nf.format(r.book.best_bid)}` : "no bids", r.book.asks ? `${nf.format(r.book.asks)} asked at ${nf.format(r.book.best_ask)}` : "no asks"].join(" · ")
        : "empty";
      const spark = document.createElementNS("http://www.w3.org/2000/svg", "svg");
      spark.setAttribute("class", "spark");
      spark.setAttribute("viewBox", "0 0 100 28");
      spark.setAttribute("preserveAspectRatio", "none");
      spark.setAttribute("aria-hidden", "true");
      // Over the whole range, so it shows when the item traded, and scaled to
      // its own lows and highs, so a small drift still shows
      drawSpark(spark, trailing(list, r.id), false);
      tr.title = r.off ? "The off-target watch is firing: a game day's VWAP is further from target than the threshold." : "";
      tr.append(
        cell(capital(item(r.id))),
        cell(nf.format(r.target), "num dim"),
        cell(price(r.vwap), "num now"),
        // Half again either way is where the watch's default threshold sits
        cell(against, `num ${by == null ? "dim" : by <= 0.5 || by >= 1.5 ? "far" : ""}`),
        cell(r.units ? `${nf.format(r.units)} in ${plural(r.trades, "fill", "fills")}` : "none", "num dim"),
        cell(book, "dim"),
        cell(spark, "trend"),
      );
      return tr;
    };
  }

  // ---------------------------------------------------------------------------
  // The pages: each story's sentence, from what the charts show

  // The overview's tiles (D1.2): each reads the newest sample that has its
  // field, says whether that's well, and draws a sparkline over the range.
  // The tick's tiles read the few numbers of `load` in the base field set
  // (#411), so their sparklines run the whole range from the first read.
  const TILES = [
    {
      key: "health",
      label: "Health",
      read() {
        const h = state.health;
        const fresh = state.lastSampleAt && Date.now() - state.lastSampleAt < 3 * SAMPLE_MS;
        if (!h) return { value: "…", sub: "Asking the server.", level: "" };
        if (h.error) return { value: "Silent", sub: "The server didn't answer /health.", level: "bad", problem: "the server didn't answer its health check" };
        const ill = [h.database !== "healthy" && "the database", h.redis !== "healthy" && "Redis"].filter(Boolean);
        if (ill.length) {
          const who = ill.join(" and ");
          return { value: "Degraded", sub: `${who.charAt(0).toUpperCase()}${who.slice(1)} ${ill.length === 1 ? "doesn't" : "don't"} answer.`, level: "bad", problem: `${who} ${ill.length === 1 ? "doesn't" : "don't"} answer` };
        }
        if (state.live === "live" && state.lastSampleAt && !fresh) {
          return { value: "Quiet", sub: `No sample since ${clock(state.lastSampleAt)}.`, level: "warn", problem: "the sampler has gone quiet" };
        }
        const scale = h.sim_tick_scale ? ` Time runs ×${h.sim_tick_scale}.` : "";
        return { value: "Sound", sub: `The database and Redis answer.${scale}`, level: "good" };
      },
    },
    {
      key: "tps",
      label: "Ticks a second",
      zero: true,
      has: (s) => s.load,
      at: (s) => (s.load.secs > 0 ? s.load.ticks / s.load.secs : null),
      read(s) {
        const tps = this.at(s);
        const want = 1000 / s.load.tick_period_ms;
        if (tps == null) return { value: "–", sub: "No window yet." };
        const low = tps < want * 0.95;
        return {
          value: tps.toFixed(1),
          sub: `Of ${Number(want.toFixed(1))}.${s.load.ticks_over ? ` ${plural(s.load.ticks_over, "tick", "ticks")} over budget.` : ""}`,
          level: low ? "warn" : "good",
          problem: low && `the tick keeps ${tps.toFixed(1)} a second of its ${Number(want.toFixed(1))}`,
        };
      },
    },
    {
      key: "p95",
      label: "Tick p95",
      zero: true,
      has: (s) => s.load,
      at: (s) => s.load.tick_p95_ms,
      read(s) {
        const { tick_p95_ms: p95, tick_period_ms: period, tick_mean_ms: mean } = s.load;
        if (p95 == null) return { value: s.load.ticks ? `> ${ms(period)}` : "–", sub: s.load.ticks ? "Past the last bucket." : "No tick ran.", level: s.load.ticks ? "bad" : "" };
        const over = p95 > period;
        return {
          value: Number(p95 < 10 ? p95.toFixed(2) : p95.toFixed(1)),
          unit: "ms",
          sub: `Of ${ms(period)}; ${ms(mean)} on average.`,
          level: over ? "bad" : "good",
          problem: over && `the tick's p95 is ${ms(p95)}, over its ${ms(period)}`,
        };
      },
    },
    {
      key: "people",
      label: "People on",
      has: (s) => s.players,
      at: (s) => s.players.people,
      read(s) {
        const sockets = latest((s) => s.load)?.load.sockets_open;
        return { value: nf.format(s.players.people), sub: sockets == null ? "In the world now." : `${plural(sockets, "socket", "sockets")} open.` };
      },
    },
    {
      key: "citizens",
      label: "Citizens on",
      has: (s) => s.players,
      at: (s) => s.players.citizens,
      read(s) {
        const t = latest((s) => s.town)?.town;
        if (!t) return { value: nf.format(s.players.citizens), sub: "No town reports in." };
        const st = t.standing ?? {};
        const roles = ["working", "resting"].filter((k) => st[k]).map((k) => `${nf.format(st[k])} ${k}`);
        const away = st.away ? `${nf.format(st.away)} away` : null;
        return { value: nf.format(s.players.citizens), sub: [...roles, away].filter(Boolean).join(", ") + "." };
      },
    },
    {
      key: "drift",
      label: "Drift",
      zero: true,
      at: (s) => (s.drift ? s.drift.strands + Object.values(s.drift.units ?? {}).reduce((a, n) => a + n, 0) : null),
      // Samples from before #358 have no `drift`; the watch still says
      has: (s) => s.drift || s.watches,
      read(s) {
        const firing = (s.watches ?? []).some((w) => w.kind === "drift");
        const d = s.drift ?? { strands: 0, units: {} };
        const units = Object.entries(d.units ?? {}).sort((a, b) => b[1] - a[1]);
        if (!firing && d.strands === 0 && units.length === 0) {
          return { value: "0", sub: "The books balance, since the server started.", level: "good" };
        }
        const parts = [];
        if (d.strands) parts.push(`${nf.format(d.strands)} ${d.strands === 1 ? "Strand" : "Strands"}`);
        for (const [i, n] of units.slice(0, 2)) parts.push(`${nf.format(n)} ${item(i)}`);
        if (units.length > 2) parts.push(`${units.length - 2} more items`);
        const what = parts.length ? parts.join(", ") : "an amount this sample doesn't carry";
        return {
          value: d.strands ? nf.format(d.strands) : units.length ? nf.format(units.reduce((a, [, n]) => a + n, 0)) : "≠ 0",
          sub: `Unexplained since the server started: ${what}. It clears on a restart.`,
          level: "bad",
          problem: `the books have drifted by ${what}`,
        };
      },
    },
  ];

  function buildTiles() {
    const box = $("tiles");
    if (box.childElementCount) return;
    box.replaceChildren(...TILES.map((t) => {
      const el = document.createElement("div");
      el.className = "tile";
      el.dataset.tile = t.key;
      const label = text("p", t.label);
      label.className = "label";
      const value = text("p", "–");
      value.className = "value";
      const spark = document.createElementNS("http://www.w3.org/2000/svg", "svg");
      spark.setAttribute("class", "spark");
      spark.setAttribute("viewBox", "0 0 100 28");
      spark.setAttribute("preserveAspectRatio", "none");
      spark.setAttribute("aria-hidden", "true");
      const sub = text("p", "");
      sub.className = "sub";
      el.append(label, value, spark, sub);
      return el;
    }));
  }

  /** A sparkline's area and line over `values`, a gap where one is null */
  function drawSpark(svg, values, zero) {
    const nums = values.filter((v) => v != null);
    svg.replaceChildren();
    if (nums.length < 2) return;
    let lo = zero ? Math.min(0, ...nums) : Math.min(...nums);
    let hi = Math.max(...nums);
    // A flat line sits low when it's zero, mid-height otherwise
    if (hi - lo < 1e-9) { if (zero && hi === 0) hi = 1; else { lo -= 1; hi += 1; } }
    const x = (i) => (values.length === 1 ? 100 : (100 * i) / (values.length - 1));
    const y = (v) => 26 - (24 * (v - lo)) / (hi - lo);
    let line = "";
    let area = "";
    let run = [];
    const close = () => {
      if (run.length > 1) {
        line += run.map(([i, v], k) => `${k ? "L" : "M"}${x(i).toFixed(2)},${y(v).toFixed(2)}`).join("");
        area += `M${x(run[0][0]).toFixed(2)},28` + run.map(([i, v]) => `L${x(i).toFixed(2)},${y(v).toFixed(2)}`).join("") + `L${x(run.at(-1)[0]).toFixed(2)},28Z`;
      }
      run = [];
    };
    values.forEach((v, i) => (v == null ? close() : run.push([i, v])));
    close();
    const path = (d, cls) => {
      const p = document.createElementNS("http://www.w3.org/2000/svg", "path");
      p.setAttribute("d", d);
      p.setAttribute("class", cls);
      p.setAttribute("vector-effect", "non-scaling-stroke");
      return p;
    };
    svg.append(path(area, "area"), path(line, "line"));
  }

  // /health is the database and Redis answering; asked at most every 30 s,
  // while the overview is open
  async function checkHealth() {
    if (state.health && Date.now() - state.health.at < 30_000) return;
    const at = Date.now();
    state.health = { ...(state.health ?? {}), at };
    try {
      const res = await fetch("/health");
      state.health = res.ok ? { at, ...(await res.json()) } : { at, error: res.status };
    } catch {
      state.health = { at, error: "unreachable" };
    }
    if (state.tab === "overview") tiles();
  }

  function tiles() {
    buildTiles();
    const problems = [];
    let any = false;
    for (const t of TILES) {
      const el = document.querySelector(`.tile[data-tile="${t.key}"]`);
      const s = t.has ? latest(t.has) : null;
      if (t.has && !s) {
        el.dataset.level = "";
        el.querySelector(".value").replaceChildren("–");
        el.querySelector(".sub").textContent = "Waiting for a sample.";
        drawSpark(el.querySelector(".spark"), []);
        continue;
      }
      if (t.has) any = true;
      const r = t.read(s);
      el.dataset.level = r.level ?? "";
      el.querySelector(".value").replaceChildren(String(r.value), ...(r.unit ? [text("small", ` ${r.unit}`)] : []));
      el.querySelector(".sub").textContent = r.sub;
      el.title = r.problem ? `${r.problem.charAt(0).toUpperCase()}${r.problem.slice(1)}.` : "";
      if (r.problem) problems.push(r.problem);
      drawSpark(el.querySelector(".spark"), t.at ? inRange(t.has).map((s) => t.at(s)) : [], t.zero);
    }
    const others = latest((s) => s.watches)?.watches.filter((w) => w.kind !== "drift" && w.kind !== "tick_p95") ?? [];
    const watchWords = { gini: "wealth is pooling", tax_revenue: "the property tax is under its floor", off_target: "an item's price is off target" };
    for (const kind of new Set(others.map((w) => w.kind))) problems.push(watchWords[kind] ?? `the ${kind.replace(/_/g, " ")} watch is firing`);
    $("glance-headline").textContent = !any
      ? (state.live === "live" ? "No samples yet: the sampler takes one every ten seconds." : "Waiting for the first sample.")
      : problems.length === 0
        ? "All's well: the server answers, the tick keeps time, and the books balance."
        : `${problems.length === 1 ? "One thing to look at" : `${problems.length} things to look at`}: ${problems.join("; ")}.`;
  }

  function overview() {
    checkHealth();
    tiles();
    latestWire();
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

  async function economy() {
    if (state.covered.money > rangeFrom() || state.covered.making > rangeFrom() || !books || !recipeNames) {
      if (!books) $("flows-headline").textContent = "Reading the books.";
      try {
        books ??= await getJson("/admin/books");
        recipeNames ??= new Map((await getJson("/recipes")).recipes.map((r) => [r.id, r.name]));
        await Promise.all([readSeries("money"), readSeries("making")]);
      } catch (e) { if (e instanceof SignedOut) return; console.warn(e); }
    }
    strands();
    if (books) flows();
    escrow();
    grants();
    distribution();
    if (books) productionStories();
    await prices();
  }

  function strands() {
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

  // How evenly people's and citizens' Strands are held (D2.2), and by which
  // trade: each citizen's from the town's report, "citizen" for one it
  // doesn't name, and "person" for people. Admins aren't counted.
  function distribution() {
    if (state.giniWatch == null) loadKnobs();
    chart("spread");
    const s = latest((s) => s.distribution);
    const d = s?.distribution;
    if (!d || d.gini == null) {
      $("spread-headline").textContent = d ? "Too few hold any Strands to say how evenly." : "Waiting for a sample that says.";
      $("roles-headline").textContent = "Waiting for a sample that says.";
      $("roles").tBodies[0].replaceChildren(emptyRow(8, "No sample says yet."));
      $("headcount-headline").textContent = "";
      return;
    }
    const tenth = Math.max(1, Math.ceil(d.accounts / 10));
    const even = d.gini < 0.3 ? "fairly evenly" : d.gini < 0.5 ? "unevenly" : d.gini < 0.7 ? "very unevenly" : "in a few hands";
    let line = `Strands are held ${even}: a Gini of ${d.gini.toFixed(2)} over ${plural(d.accounts, "account", "accounts")}, and the richest ${plural(tenth, "holds", "hold")} ${percent(d.top_tenth)} of them.`;
    const first = inRange((s) => s.distribution?.gini != null)[0];
    if (first && first !== s) {
      const was = first.distribution.gini;
      const moved = d.gini - was;
      line += Math.abs(moved) < 0.005 ? ` Steady ${since(first)}.` : ` ${moved > 0 ? "Up" : "Down"} from ${was.toFixed(2)} ${since(first)}.`;
    }
    if (state.giniWatch != null && d.gini > state.giniWatch) line += ` That's over the watch's ${state.giniWatch.toFixed(2)}.`;
    $("spread-headline").textContent = line;
    roles(d, inRange((s) => s.distribution).map((s) => s.distribution));
  }

  // Callings K3.3: each job by its slug, as the town's report names it,
  // with its activity (time by activity's key, K0.1) and what doing it is,
  // as the citizens' own words have them (calling::doing)
  const JOBS = {
    "gatherer-fiber": { activity: "fiber", doing: "cutting grass" },
    "gatherer-wood": { activity: "wood", doing: "felling trees" },
    "gatherer-ore": { activity: "ore", doing: "mining ore" },
    "crafter-planks": { activity: "planks", doing: "sawing planks" },
    "crafter-ingots": { activity: "ingots", doing: "smelting metal" },
    "crafter-tools": { activity: "tools", doing: "making tools" },
    builder: { activity: "building", doing: "building" },
    merchant: { activity: "trading", doing: "trading" },
  };
  // What a citizen's time goes on (K0.1), in the reports' order. Each has
  // its ink, --a-<key>
  const ACTIVITIES = [
    ["fiber", "Fiber"], ["wood", "Wood"], ["ore", "Ore"], ["planks", "Planks"], ["ingots", "Ingots"], ["tools", "Tools"],
    ["building", "Building"], ["trading", "Trading"], ["commons", "The Commons"], ["detour", "Detours"], ["road_in", "The road in"], ["free", "Free"],
  ];
  const ACTIVITY = Object.fromEntries(ACTIVITIES);
  /** An activity's name inside a sentence: "wood", "the Commons" */
  const lc = (name) => (name === "The Commons" ? "the Commons" : name.toLowerCase());
  /** A job by its slug, as its activity: "Wood", "Building" */
  const jobName = (slug) => ACTIVITY[JOBS[slug]?.activity] ?? capital(slug.replace(/^([a-z]+)-(.+)$/, "$1 ($2)"));

  // A bucket of the wealth table: each citizen's favourite job, all-rounders
  // together (Callings K3.3; before callings, its trade, which is the same
  // key), and people together
  const trade = (role) => role === "person" ? "People" : role === "citizen" ? "Citizens with no calling named" : role === "all-rounder" ? "All-rounders" : jobName(role);
  /** A group's balances with its escrow (#559): what it holds, all told */
  const held = (h) => h.strands + escrowOf(h);
  const escrowOf = (h) => (h.escrow?.bids ?? 0) + (h.escrow?.commissions ?? 0) + (h.escrow?.wages ?? 0);
  const each = (h) => held(h) / Math.max(h.accounts, 1);
  /** One of a bucket, in a sentence: "a person", "an all-rounder", "a citizen who favours wood" */
  const one = (role) => role === "person" ? "a person" : role === "citizen" ? "a citizen with no calling named" : role === "all-rounder" ? "an all-rounder" : `a citizen who favours ${jobName(role).toLowerCase()}`;
  /** All of a bucket, to start a sentence: "People", "All-rounders", "Those who favour wood" */
  const those = (role) => role === "person" || role === "citizen" || role === "all-rounder" ? trade(role) : `Those who favour ${jobName(role).toLowerCase()}`;
  /** The same, after a semicolon */
  const thoseAfter = (role) => those(role).charAt(0).toLowerCase() + those(role).slice(1);

  /** A citizen's calling, briefly: "Wood, also ore and planks", "All-rounder"; its trade from a town before callings */
  function callingShort(c) {
    const k = c.calling;
    if (!k) return trade(c.role);
    if (!k.favourite) return "All-rounder";
    const also = (k.leanings ?? []).map((l) => jobName(l.job).toLowerCase());
    return also.length ? `${jobName(k.favourite)}, also ${andList(also)}` : jobName(k.favourite);
  }

  /** A citizen's calling in a sentence: "a wood specialist who also does ore and planks", "an all-rounder" */
  function callingSays(c) {
    const k = c.calling;
    if (!k) return one(c.role);
    if (!k.favourite) return "an all-rounder";
    const fav = jobName(k.favourite).toLowerCase();
    const also = andList((k.leanings ?? []).map((l) => jobName(l.job).toLowerCase()));
    if (k.style === "specialist") return `${/^[aeiou]/.test(fav) ? "an" : "a"} ${fav} specialist${also ? ` who also does ${also}` : ""}`;
    return `a citizen who leans to ${fav}${also ? `, and to ${also} too` : ""}`;
  }

  /** The job of the moment, in words: "sawing planks"; null before its first stint */
  const stintWords = (c) => (c.doing ? JOBS[c.doing.job]?.doing ?? c.doing.job : null);

  /** A stint's game hours, in words: "40 minutes", "an hour", "2.5 hours" */
  function stintFor(hours) {
    const minutes = Math.round(hours * 60);
    if (minutes < 1) return "a moment";
    if (minutes < 55) return plural(minutes, "minute", "minutes");
    if (minutes < 75) return "an hour";
    return `${(Math.round(hours * 2) / 2).toLocaleString()} hours`;
  }

  /** Its work by activity: each activity's hours and share of the work, the biggest first in `sorted`, and free time's share of all of it (K0.1) */
  function timeShares(time) {
    const work = ACTIVITIES.filter(([k]) => k !== "free" && time?.[k] > 0).map(([k, name]) => ({ key: k, name, hours: time[k] }));
    const total = work.reduce((n, a) => n + a.hours, 0);
    for (const a of work) a.share = total ? a.hours / total : 0;
    const free = time?.free ?? 0;
    return { work, total, free: total + free > 0 ? free / (total + free) : null, sorted: [...work].sort((a, b) => b.hours - a.hours) };
  }

  /** A roster's time by activity: a bar of the work's shares in the activities' inks, and the biggest in words */
  function timeCell(c) {
    const t = timeShares(c.time);
    if (!t.total) return "–";
    const bar = document.createElement("span");
    bar.className = "timebar";
    for (const a of t.work) {
      const seg = document.createElement("i");
      seg.style.flexGrow = String(a.hours);
      seg.style.background = `var(--a-${a.key})`;
      bar.append(seg);
    }
    const said = capital(`${andList(t.sorted.map((a) => `${lc(a.name)} ${percent(a.share)}`))} of its work${t.free != null ? `; free ${percent(t.free)} of its time` : ""}`);
    bar.title = said;
    bar.setAttribute("role", "img");
    bar.setAttribute("aria-label", said);
    const top = t.sorted[0];
    const words = text("span", `${lc(top.name)} ${percent(top.share)}`);
    words.className = "dim";
    return [bar, words];
  }

  // The styles' rows (#559), as the town's report names them; a style the
  // page has no words for is shown as it's named
  const STYLE_NAMES = {
    specialist: ["Specialists", "a specialist"],
    leaner: ["Leaners", "a leaner"],
    "all-rounder": ["All-rounders", "an all-rounder"],
    saver: ["Savers", "a saver"],
    person: ["People", "a person"],
    citizen: ["Citizens with no style named", "a citizen with no style named"],
  };
  /** A group of the table, as the switch has it: "Wood", "Specialists" */
  const groupName = (key) => wealthView.by === "styles" ? STYLE_NAMES[key]?.[0] ?? capital(key) : trade(key);
  /** All of a group, to start a sentence */
  const groupThose = (key) => wealthView.by === "styles" ? groupName(key) : those(key);
  // The savers (#556), once they're a style, lead the headline
  const LEAD_STYLE = "saver";
  // A group past this, either way, is named in the headline
  const FAR = 2;

  // Each group's share of what's held, balances and escrow, over its share
  // of the accounts: 1 is even. `null` while nobody holds anything. Kept
  // per distribution, since the chart asks again on every redraw
  const againstMemo = { roles: new WeakMap(), styles: new WeakMap() };
  function against(d, by) {
    const memo = againstMemo[by];
    if (memo.has(d)) return memo.get(d);
    const groups = d[by] ?? {};
    const accounts = Object.values(groups).reduce((n, h) => n + h.accounts, 0);
    const total = Object.values(groups).reduce((n, h) => n + Math.max(held(h), 0), 0);
    const out = {};
    for (const [key, h] of Object.entries(groups)) {
      const share = total ? Math.max(held(h), 0) / total : null;
      const heads = accounts ? h.accounts / accounts : 0;
      out[key] = { share, heads, ratio: share == null || !heads ? null : share / heads };
    }
    memo.set(d, out);
    return out;
  }
  /** How far a ratio is from even, either way: 2× and ½× are as far */
  const farness = (r) => (r == null ? -1 : r > 0 ? Math.abs(Math.log(r)) : Infinity);
  const times = (r) => `${r < 10 ? r.toFixed(1) : Math.round(r)}×`;

  // The wealth table: each group's accounts and Strands now, against the
  // first sample in the range that has the group (the town names them only
  // once it reports in), as the soak report's table of trades has them
  function roles(d, inRange) {
    const by = wealthView.by;
    $("roles-group").textContent = by === "styles" ? "Style" : "Calling";
    const groups = d[by] ?? {};
    const rows = Object.entries(groups).sort((a, b) => held(b[1]) - held(a[1]));
    if (!rows.length) {
      $("roles-headline").textContent = by === "styles" ? "No sample in the range names the styles yet: the town names them since #559." : "No sample says yet.";
      $("roles").tBodies[0].replaceChildren(emptyRow(8, "No sample says yet."));
      $("headcount-headline").textContent = "";
      return headcountChart([]);
    }
    const ratios = against(d, by);
    const escrow = rows.reduce((n, [, h]) => n + escrowOf(h), 0);
    const all = rows.reduce((n, [, h]) => n + Math.max(held(h), 0), 0);
    $("roles-headline").textContent = headline(rows, ratios, escrow, all);
    $("roles").tBodies[0].replaceChildren(...rows.map(([key, h]) => {
      const before = inRange.find((d) => d[by]?.[key])?.[by][key];
      const change = before ? each(h) - each(before) : null;
      const r = ratios[key];
      const tr = document.createElement("tr");
      const inEscrow = cell(escrowOf(h) ? nf.format(escrowOf(h)) : "–", `num ${escrowOf(h) ? "" : "dim"}`);
      if (escrowOf(h)) inEscrow.title = escrowSays(h);
      tr.append(
        cell(groupName(key)),
        cell(nf.format(h.accounts), "num"),
        cell(nf.format(h.strands), "num"),
        inEscrow,
        cell(nf.format(Math.round(each(h))), "num"),
        cell(change == null ? "–" : `${change > 0 ? "+" : change < 0 ? "−" : "±"}${nf.format(Math.abs(Math.round(change)))}`, `num ${change > 0 ? "up" : change < 0 ? "down" : "dim"}`),
        cell(r.share == null ? "–" : percent(r.share), "num dim"),
        cell(r.ratio == null ? "–" : times(r.ratio), `num against ${r.ratio == null ? "dim" : r.ratio >= FAR ? "far over" : r.ratio <= 1 / FAR ? "far under" : ""}`),
      );
      return tr;
    }));
    headcountChart(rows.map(([key]) => key).filter((key) => ratios[key].ratio != null).sort((a, b) => farness(ratios[b].ratio) - farness(ratios[a].ratio)));
  }

  /** What a group's escrow is: "1,200 in bids, 40 in commissions" */
  function escrowSays(h) {
    const parts = [["bids", "bids"], ["commissions", "commissions"], ["wages", "wage offers"]]
      .filter(([k]) => h.escrow?.[k]).map(([k, say]) => `${nf.format(h.escrow[k])} in ${say}`);
    return `${capital(andList(parts))}.`;
  }

  // The savers' distance from even first, once they're a style (#556);
  // otherwise the group furthest from it. Then anyone else over 2×, and how
  // much of it sits in escrow
  function headline(rows, ratios, escrow, all) {
    const ranked = rows.map(([key]) => key).filter((key) => ratios[key].ratio != null).sort((a, b) => farness(ratios[b].ratio) - farness(ratios[a].ratio));
    if (!ranked.length) return "Nobody holds a Strand yet.";
    const lead = wealthView.by === "styles" && ratios[LEAD_STYLE]?.ratio != null ? LEAD_STYLE : ranked[0];
    const r = ratios[lead];
    const even = (key) => farness(ratios[key].ratio) < 0.05;
    const tail = escrow > 0 && all > 0 ? ` ${percent(escrow / all)} of it sits in escrow.` : "";
    if (ranked.every(even)) return `Every ${wealthView.by === "styles" ? "style" : "calling"} holds about even with its headcount.${tail}`;
    const where = even(lead) ? "about even with their headcount" : `${times(r.ratio)} their headcount's share`;
    let line = `${groupThose(lead)} hold ${where}: ${percent(r.share)} of the Strands with ${percent(r.heads)} of the accounts.`;
    const others = ranked.filter((key) => key !== lead && ratios[key].ratio >= FAR);
    if (others.length) {
      const named = others.slice(0, 3).map((key) => `${groupName(key).toLowerCase()} at ${times(ratios[key].ratio)}`);
      line += ` Also over ${FAR}×: ${andList(named)}${others.length > 3 ? `, and ${others.length - 3} more` : ""}.`;
    } else if (ratios[lead].ratio < FAR) {
      line += ` Nobody is over ${FAR}×.`;
    }
    return line + tail;
  }

  // The chart over time draws the groups furthest from even now, at most
  // one per series colour; a new set of them rebuilds it
  const HEADCOUNT_LINES = 5;
  function headcountChart(ranked) {
    const lines = ranked.slice(0, HEADCOUNT_LINES);
    const changed = lines.join() !== wealthView.lines.join();
    wealthView.lines = lines;
    $("headcount-headline").textContent = lines.length
      ? `Over time, the ${lines.length === 1 ? "group" : `${lines.length} groups`} furthest from even now, by their share of the Strands over their share of the accounts.`
      : "";
    if (changed) state.charts.headcount?.rebuild();
    chart("headcount");
  }

  function rolesControls() {
    const group = $("roles-by");
    for (const b of group.querySelectorAll("button")) b.setAttribute("aria-pressed", String(b.dataset.by === wealthView.by));
    group.addEventListener("click", (e) => {
      const b = e.target.closest("button[data-by]");
      if (!b) return;
      wealthView.by = b.dataset.by;
      prefs.set("observatory.wealth.by", wealthView.by);
      for (const x of group.querySelectorAll("button")) x.setAttribute("aria-pressed", String(x === b));
      // The same keys can name other groups: all-rounders are in both
      wealthView.lines = [];
      state.charts.headcount?.rebuild();
      distribution();
    });
  }

  // ---------------------------------------------------------------------------
  // The money (D2.1): faucets and sinks by kind, binned over the range;
  // escrow; and velocity, trade over the supply

  // How the server classes each transaction type (`GET /admin/books`): the
  // page holds no copy of what's a faucet and what's a sink
  let books = null;

  // The lines the flows are drawn in. Several types can share one; a type
  // the page has no words for is drawn on its own, on its sign's side
  const LINE_OF = {
    exchange_deposit: "deposits",
    deed_refund: "refunds",
    foreclosure_refund: "refunds",
    bazaar_refund: "refunds",
    admin_adjustment: "grants",
    exchange_withdraw: "withdrawals",
    deed_purchase: "deeds",
    floor_permit: "deeds",
    tax_payment: "tax",
    property_tax: "tax",
    market_fee: "fees",
    bazaar_rent: "rent",
  };
  const LINES = {
    deposits: { label: "Deposits", say: "labour's deposits at the Exchange" },
    refunds: { label: "Refunds", say: "refunds" },
    grants: { label: "Grants", say: "admins' grants" },
    newcomers: { label: "Newcomers", say: "newcomers' first Strands" },
    withdrawals: { label: "Withdrawals", say: "Strands changed back at the Exchange" },
    deeds: { label: "Deeds", say: "deeds and floor permits" },
    rent: { label: "Stall rent", say: "stall rent" },
    fees: { label: "Market fees", say: "the market's fees" },
    tax: { label: "Tax", say: "tax" },
  };
  // A bar is five minutes of the last hour, a quarter of the last six, an
  // hour of the last day: about two dozen bars at any range
  const BIN_MINUTES = { 1: 5, 6: 15, 24: 60 };

  let money = { lines: [], rows: null, velocity: null, key: "" };

  /** The server's class for a transaction type */
  const classOf = (kind) => books.flows.find((f) => f.kind === kind)?.flow ?? "other";

  /** Which line a transaction type is drawn in, or null for one that only moves Strands about */
  function lineOf(kind) {
    const flow = classOf(kind);
    if (flow === "transfer" || flow === "escrow") return null;
    return LINE_OF[kind] ?? kind;
  }

  /** The range's samples in bins, each with its flows by line, its trade and the supply */
  function binMoney() {
    const binMs = BIN_MINUTES[state.hours] * 60_000;
    const first = Math.floor(rangeFrom() / binMs) * binMs;
    const bins = Array.from({ length: Math.ceil((Date.now() - first) / binMs) }, (_, k) => ({
      at: first + k * binMs, ms: 0, flows: {}, market: 0, bazaar: 0, hand: 0, supply: 0, supplies: 0,
    }));
    const shown = inRange((s) => s.strand_flows);
    const sumOf = (trades) => Object.values(trades ?? {}).reduce((a, t) => a + t.strands, 0);
    for (const s of shown) {
      const b = bins[Math.floor((s.at_ms - first) / binMs)];
      if (!b) continue;
      b.ms += s.since_ms;
      for (const [kind, v] of Object.entries(s.strand_flows)) {
        const line = lineOf(kind);
        if (line) b.flows[line] = (b.flows[line] ?? 0) + v;
      }
      if (s.new_accounts) b.flows.newcomers = (b.flows.newcomers ?? 0) + s.new_accounts * books.starting_strands;
      b.market += sumOf(s.market);
      b.bazaar += sumOf(s.bazaar);
      // A trade between players is two rows that net to nothing; what was
      // received is what changed hands
      b.hand += Math.max(0, s.strand_flows.trade_received ?? 0);
      if (s.strands) {
        b.supply += Object.values(s.strands).reduce((a, n) => a + n, 0);
        b.supplies++;
      }
    }
    return { binMs, bins, shown };
  }

  /** The lines with a flow in the range: what came in, then what went out, each in the server's order */
  function moneyLines(totals) {
    const order = [...new Set([...books.flows.map((f) => lineOf(f.kind)).filter(Boolean), "newcomers"])];
    const keys = [...order, ...Object.keys(totals).filter((l) => !order.includes(l))].filter((l) => totals[l]);
    const faucet = (l) =>
      l === "newcomers" || (LINES[l] ? books.flows.some((f) => lineOf(f.kind) === l && f.flow === "faucet") : totals[l] > 0);
    return [...keys.filter(faucet), ...keys.filter((l) => !faucet(l))].map((key) => ({
      key,
      side: faucet(key) ? "in" : "out",
      label: LINES[key]?.label ?? capital(item(key)),
      say: LINES[key]?.say ?? item(key),
      color: LINES[key] ? `--fl-${key}` : "--fl-other",
    }));
  }

  function flows() {
    const { binMs, bins, shown } = binMoney();
    const totals = {};
    for (const b of bins) for (const [l, v] of Object.entries(b.flows)) totals[l] = (totals[l] ?? 0) + v;
    const lines = moneyLines(totals);

    // Each bar stacks up from zero for what came in and down for what went
    // out, so each line is drawn to its running total
    const values = {};
    const own = {};
    for (const l of lines) {
      values[l.key] = [];
      own[l.key] = [];
    }
    for (const b of bins) {
      let up = 0;
      let down = 0;
      for (const l of lines) {
        const v = b.flows[l.key] ?? 0;
        own[l.key].push(b.ms ? v : null);
        if (!b.ms || v === 0) values[l.key].push(null);
        else values[l.key].push(v > 0 ? (up += v) : (down += v));
      }
    }
    const x = bins.map((b) => (b.at + binMs / 2) / 1000);
    // How many times the money would change hands in a game day at the bin's pace
    const velocity = bins.map((b) => {
      if (!b.ms || !b.supplies) return null;
      const supply = b.supply / b.supplies;
      return supply > 0 ? ((b.market + b.bazaar + b.hand) / supply) * (books.day_ms / b.ms) : null;
    });

    const key = lines.map((l) => l.key).join();
    const rebuild = key !== money.key;
    money = { lines, key, rows: { x, shown, values, own }, velocity: { x, shown, values: { velocity } } };
    if (rebuild) for (const name of ["flows", "tune-sinks"]) state.charts[name]?.rebuild();
    chart("flows");
    chart("velocity");
    flowsHeadline(shown, lines, totals);
    account(lines, totals);
    velocityHeadline(shown, bins);
  }

  /** "a, b and c" */
  const andList = (parts) => (parts.length < 2 ? parts.join("") : `${parts.slice(0, -1).join(", ")} and ${parts.at(-1)}`);

  function flowsHeadline(shown, lines, totals) {
    const h = $("flows-headline");
    if (!shown.length) return void (h.textContent = "Waiting for the first sample.");
    const side = (s, sign) => lines.filter((l) => l.side === s).map((l) => [l, sign * totals[l.key]]).sort((a, b) => b[1] - a[1]);
    const ins = side("in", 1);
    const outs = side("out", -1);
    const came = ins.reduce((a, [, v]) => a + v, 0);
    const went = outs.reduce((a, [, v]) => a + v, 0);
    const when = capital(since(shown[0]));
    if (came === 0 && went === 0) return void (h.textContent = `${when}, no Strands came into the city or left it.`);
    let line = `${when}, ${plural(came, "Strand", "Strands")} came in`;
    if (ins.length > 1) line += `, ${Math.round((100 * ins[0][1]) / came)}% of them ${ins[0][0].say}`;
    else if (ins.length) line += `, all ${ins[0][0].say}`;
    line += went === 0 ? ", and none went out." : `, and ${nf.format(went)} went out: ${andList(outs.map(([l, v]) => `${l.say} ${nf.format(v)}`))}.`;
    // Whether tax bites (#335) is what this chart was made to show
    if (went > 0) {
      const tax = outs.find(([l]) => l.key === "tax")?.[1] ?? 0;
      const share = (100 * tax) / went;
      line += tax === 0
        ? " No tax was paid."
        : outs.length > 1 ? ` Tax took ${share < 1 ? "under 1%" : `${Math.round(share)}%`} of it.` : "";
    }
    h.textContent = line;
  }

  /** The range's account: each line's total as a bar, and what it nets to */
  function account(lines, totals) {
    const box = $("money-account");
    if (!lines.length) return void box.replaceChildren();
    const most = Math.max(...lines.map((l) => Math.abs(totals[l.key])));
    const signed = (v) => `${v > 0 ? "+" : v < 0 ? "−" : ""}${nf.format(Math.abs(v))}`;
    const row = (l) => {
      const v = totals[l.key];
      const bar = document.createElement("span");
      bar.className = "account-bar";
      // Never thinner than a hairline, so a Strand of tax still shows
      bar.style.width = `max(2px, ${(100 * Math.abs(v)) / most}%)`;
      bar.style.background = `var(${l.color})`;
      const track = document.createElement("span");
      track.className = "account-track";
      track.append(bar);
      const r = document.createElement("div");
      r.className = "account-row";
      r.dataset.side = l.side;
      r.append(text("span", l.label), track, text("span", signed(v)));
      return r;
    };
    const net = lines.reduce((a, l) => a + totals[l.key], 0);
    const foot = document.createElement("div");
    foot.className = "account-row account-net";
    const says = net === 0 ? "The city's Strands held level." : `The city has ${nf.format(Math.abs(net))} ${net > 0 ? "more" : "fewer"} than it had.`;
    foot.append(text("span", "Net"), text("span", says), text("span", signed(net)));
    box.replaceChildren(...lines.map(row), foot);
  }

  function velocityHeadline(shown, bins) {
    const h = $("velocity-headline");
    const live = bins.filter((b) => b.ms && b.supplies);
    if (!live.length) return void (h.textContent = "Waiting for the first sample.");
    const ms = live.reduce((a, b) => a + b.ms, 0);
    const supply = live.reduce((a, b) => a + b.supply / b.supplies, 0) / live.length;
    const [market, bazaar, hand] = ["market", "bazaar", "hand"].map((k) => live.reduce((a, b) => a + b[k], 0));
    const traded = market + bazaar + hand;
    if (traded === 0) return void (h.textContent = `Nothing changed hands ${since(shown[0])}: the money sat still.`);
    const v = supply > 0 ? (traded / supply) * (books.day_ms / ms) : 0;
    const pct = (n) => `${Math.round((100 * n) / traded)}%`;
    const where = [market && `${pct(market)} on the market`, bazaar && `${pct(bazaar)} at the bazaar`, hand && `${pct(hand)} hand to hand`].filter(Boolean);
    const times = v < 0.01 ? "less than a hundredth of a time" : `${v.toFixed(2)} times`;
    h.textContent = `At the pace of ${window_()}, the city's money changes hands ${times} a game day: ${plural(traded, "Strand", "Strands")} traded, ${andList(where)}.`;
  }

  function escrow() {
    chart("escrow");
    const h = $("escrow-headline");
    const s = latest((s) => s.strands);
    if (!s) return void (h.textContent = "Waiting for the first sample.");
    const held = s.strands.escrow;
    const total = Object.values(s.strands).reduce((a, b) => a + b, 0);
    const most = Math.max(held, ...inRange((s) => s.strands).map((s) => s.strands.escrow));
    if (most === 0) return void (h.textContent = `No buy order held a Strand in ${window_()}.`);
    let line = held === 0
      ? "No buy order holds a Strand now."
      : `Buy orders hold ${plural(held, "Strand", "Strands")}, ${((100 * held) / total).toFixed(1)}% of the money.`;
    if (most > held) line += ` At most ${nf.format(most)} in ${window_()}.`;
    h.textContent = line;
  }

  /** The road in's deed grants (#440): land given, not bought, outside the Strands' books */
  function grants() {
    const h = $("grants-headline");
    const s = latest((s) => s.grants);
    if (!s) return void (h.textContent = "Waiting for the first sample.");
    const { opened, used, returned, granted_value: value } = s.grants;
    if (opened === 0) return void (h.textContent = "Nobody has walked the road in to their deed yet.");
    let line = `${plural(opened, "deed has", "deeds have")} been earned on the road in`;
    line += used === 0
      ? ", and none is spent yet."
      : `, and ${nf.format(used)} ${used === 1 ? "holds a lot" : "hold lots"} assessed at ${plural(value, "Strand", "Strands")}.`;
    if (returned) line += ` ${plural(returned, "was", "were")} handed back by a retired lot.`;
    h.textContent = line;
  }

  // ---------------------------------------------------------------------------
  // Production (D2.4): units made and used by how, binned as the money is;
  // each item's rates and stock; and each recipe's crafts and quality out

  // How units come and go (`UnitFlow`), in the order they stack: what
  // brings units in, then what takes them out
  const UNIT_FLOWS = {
    gathered: { label: "Gathered", say: "gathered" },
    crafted: { label: "Crafted", say: "crafted" },
    withdrawn: { label: "Withdrawn", say: "withdrawn from the Exchange" },
    kit_returned: { label: "Kits back", say: (n) => `taken down to ${n === 1 ? "a kit" : "kits"}` },
    granted: { label: "Granted", say: "admins' grants" },
    consumed: { label: "Used in crafts", say: "used in crafts" },
    worn_out: { label: "Worn out", say: "tools worn out" },
    deposited: { label: "Deposited", say: "deposited at the Exchange" },
    kit_placed: { label: "Kits placed", say: (n) => `set up as ${n === 1 ? "a station" : "stations"}` },
    lost: { label: "Lost", say: "lost with foreclosed or fallen stations" },
    repaired: { label: "Repairs", say: "used repairing stations" },
    built: { label: "Floors", say: "built into new floors" },
  };
  // The items the stock table names; the quieter ones are counted under it
  const STOCK_ROWS = 20;
  // A sparkline needs no more points than it has pixels
  const SPARK_POINTS = 120;

  // The recipes' names (`GET /recipes`), read once
  let recipeNames = null;
  let making = { lines: [], rows: null, key: "" };

  /** `n` for every game hour of `ms` */
  const perGameHour = (n, ms) => (ms > 0 ? (n * books.day_ms) / 24 / ms : 0);
  const rate = (v) => (v === 0 ? "–" : v < 10 ? (Math.round(v * 10) / 10).toLocaleString("en") : count(v));
  const signed = (v) => `${v > 0 ? "+" : v < 0 ? "−" : ""}${nf.format(Math.abs(v))}`;

  /** The range's unit flows in the money's bins, and its totals by item and by recipe */
  function production() {
    const binMs = BIN_MINUTES[state.hours] * 60_000;
    const first = Math.floor(rangeFrom() / binMs) * binMs;
    const bins = Array.from({ length: Math.ceil((Date.now() - first) / binMs) }, (_, k) => ({ at: first + k * binMs, ms: 0, flows: {} }));
    const shown = inRange((s) => s.unit_flows);
    const items = {};
    const recipes = {};
    let ms = 0;
    for (const s of shown) {
      ms += s.since_ms;
      const b = bins[Math.floor((s.at_ms - first) / binMs)];
      if (b) b.ms += s.since_ms;
      for (const [it, byFlow] of Object.entries(s.unit_flows)) {
        const row = (items[it] ??= { made: 0, used: 0 });
        for (const [flow, n] of Object.entries(byFlow)) {
          if (n > 0) row.made += n;
          else row.used -= n;
          if (b) b.flows[flow] = (b.flows[flow] ?? 0) + n;
        }
      }
      for (const [id, c] of Object.entries(s.crafts ?? {})) {
        const r = (recipes[id] ??= { runs: 0, units: 0, quality: 0 });
        r.runs += c.runs;
        r.units += c.units;
        r.quality += c.quality;
      }
    }
    return { binMs, bins, shown, items, recipes, ms };
  }

  function madeAndUsed(p) {
    const totals = {};
    for (const b of p.bins) for (const [f, v] of Object.entries(b.flows)) totals[f] = (totals[f] ?? 0) + v;
    const order = [...Object.keys(UNIT_FLOWS), ...Object.keys(totals).filter((f) => !UNIT_FLOWS[f])];
    const lines = order.filter((f) => totals[f]).map((key) => ({
      key,
      side: totals[key] > 0 ? "in" : "out",
      label: UNIT_FLOWS[key]?.label ?? capital(item(key)),
      say: (n) => { const say = UNIT_FLOWS[key]?.say ?? item(key); return typeof say === "function" ? say(n) : say; },
      color: UNIT_FLOWS[key] ? `--uf-${key}` : "--fl-other",
    }));
    // Stacked up from zero for what came in and down for what went out, as
    // the money's bars are
    const values = {};
    const own = {};
    for (const l of lines) {
      values[l.key] = [];
      own[l.key] = [];
    }
    for (const b of p.bins) {
      let up = 0;
      let down = 0;
      for (const l of lines) {
        const v = b.flows[l.key] ?? 0;
        own[l.key].push(b.ms ? v : null);
        if (!b.ms || v === 0) values[l.key].push(null);
        else values[l.key].push(v > 0 ? (up += v) : (down += v));
      }
    }
    const x = p.bins.map((b) => (b.at + p.binMs / 2) / 1000);
    const key = lines.map((l) => l.key).join();
    const rebuild = key !== making.key;
    making = { lines, key, rows: { x, shown: p.shown, values, own } };
    if (rebuild) for (const name of ["making", "tune-production"]) state.charts[name]?.rebuild();
    chart("making");

    const h = $("making-headline");
    if (!p.shown.length) return void (h.textContent = "Waiting for the first sample.");
    const side = (s, sign) => lines.filter((l) => l.side === s).map((l) => [l, sign * totals[l.key]]).sort((a, b) => b[1] - a[1]);
    const ins = side("in", 1);
    const outs = side("out", -1);
    const came = ins.reduce((a, [, v]) => a + v, 0);
    const went = outs.reduce((a, [, v]) => a + v, 0);
    const when = capital(since(p.shown[0]));
    if (came === 0 && went === 0) return void (h.textContent = `${when}, nothing was made and nothing used up.`);
    let line = `${when}, ${plural(came, "unit", "units")} came into the world`;
    if (ins.length > 1) line += `, ${Math.round((100 * ins[0][1]) / came)}% of them ${ins[0][0].say(2)}`;
    else if (ins.length) line += `, all ${ins[0][0].say(2)}`;
    line += went === 0 ? ", and none left it." : `, and ${nf.format(went)} left it: ${andList(outs.map(([l, v]) => `${nf.format(v)} ${l.say(v)}`))}.`;
    h.textContent = line;
  }

  function stock(p) {
    const now = latest((s) => s.units);
    const body = $("stock").tBodies[0];
    const h = $("stock-headline");
    if (!now) {
      h.textContent = "Waiting for the first sample.";
      return void body.replaceChildren(emptyRow(6, "No sample has counted the units yet."));
    }
    const runs = inRange((s) => s.units);
    const step = Math.max(1, Math.ceil(runs.length / SPARK_POINTS));
    const thinned = runs.filter((_, i) => i % step === 0 || i === runs.length - 1);
    const start = runs[0] ?? now;
    const rows = [...new Set([...Object.keys(now.units), ...Object.keys(p.items)])].map((it) => {
      const flow = p.items[it] ?? { made: 0, used: 0 };
      const held = now.units[it] ?? 0;
      return {
        it,
        made: perGameHour(flow.made, p.ms),
        used: perGameHour(flow.used, p.ms),
        busy: flow.made + flow.used,
        held,
        change: held - (start.units[it] ?? 0),
      };
    }).sort((a, b) => b.busy - a.busy || b.held - a.held || a.it.localeCompare(b.it));

    const trs = rows.slice(0, STOCK_ROWS).map((r) => {
      const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
      svg.setAttribute("viewBox", "0 0 100 28");
      svg.setAttribute("preserveAspectRatio", "none");
      svg.setAttribute("aria-label", `${capital(item(r.it))}'s stock over ${window_()}`);
      drawSpark(svg, thinned.map((s) => s.units[r.it] ?? 0), true);
      const tr = document.createElement("tr");
      tr.append(
        cell(capital(item(r.it))),
        cell(rate(r.made), "num"),
        cell(rate(r.used), "num"),
        cell(nf.format(r.held), "num"),
        cell(r.change === 0 ? "–" : signed(r.change), `num ${r.change > 0 ? "up" : r.change < 0 ? "down" : "dim"}`),
        cell(svg, "trend"),
      );
      return tr;
    });
    const quiet = rows.slice(STOCK_ROWS);
    if (quiet.length) {
      const tr = document.createElement("tr");
      tr.className = "more";
      const td = cell(`And ${plural(quiet.length, "quieter item", "quieter items")}, holding ${plural(quiet.reduce((a, r) => a + r.held, 0), "unit", "units")} between them.`);
      td.colSpan = 6;
      tr.append(td);
      trs.push(tr);
    }
    body.replaceChildren(...trs);

    const busiest = rows.find((r) => r.made > 0);
    if (!busiest) {
      const held = rows.reduce((a, r) => a + r.held, 0);
      h.textContent = `Nothing was made ${since(start)}. The city holds ${plural(held, "unit", "units")} of ${plural(rows.filter((r) => r.held).length, "item", "items")}.`;
      return;
    }
    const grew = rows.filter((r) => r.change > 0).sort((a, b) => b.change - a.change)[0];
    const fell = rows.filter((r) => r.change < 0).sort((a, b) => a.change - b.change)[0];
    let line = `${capital(item(busiest.it))} was made most, ${rate(busiest.made)} a game hour`;
    if (grew === busiest) line += `, and piled up most, ${signed(grew.change)}`;
    const moved = [grew !== busiest && grew && `${item(grew.it)} piled up most, ${signed(grew.change)}`, fell && `${item(fell.it)} ran down most, ${signed(fell.change)}`].filter(Boolean);
    h.textContent = line + (moved.length ? `; ${andList(moved)}.` : grew || fell ? "." : ", and no stock changed.");
  }

  function workshops(p) {
    const body = $("recipes").tBodies[0];
    const h = $("recipes-headline");
    const name = (id) => recipeNames?.get(id) ?? capital(item(id));
    if (!p.shown.length) {
      h.textContent = "Waiting for the first sample.";
      return void body.replaceChildren(emptyRow(5, "No sample yet."));
    }
    const rows = Object.entries(p.recipes).map(([id, r]) => ({ id, ...r })).sort((a, b) => b.runs - a.runs || a.id.localeCompare(b.id));
    if (!rows.length) {
      h.textContent = `No craft finished ${since(p.shown[0])}.`;
      return void body.replaceChildren(emptyRow(5, `Nothing was crafted in ${window_()}.`));
    }
    body.replaceChildren(...rows.map((r) => {
      const q = r.quality / r.runs;
      const bar = document.createElement("span");
      bar.className = "account-bar";
      bar.style.width = `${Math.max(1, Math.min(100, q))}%`;
      const track = document.createElement("span");
      track.className = "account-track";
      track.append(bar);
      const grade = document.createElement("span");
      grade.className = "grade";
      grade.title = `Q${q.toFixed(1)}`;
      grade.append(track, text("span", String(Math.round(q))));
      const tr = document.createElement("tr");
      tr.append(
        cell(name(r.id)),
        cell(nf.format(r.runs), "num"),
        cell(rate(perGameHour(r.runs, p.ms)), "num"),
        cell(nf.format(r.units), "num"),
        cell(grade),
      );
      return tr;
    }));
    const runs = rows.reduce((a, r) => a + r.runs, 0);
    const top = rows[0];
    let line = `${name(top.id)} was crafted most: ${plural(top.runs, "time", "times")} ${since(p.shown[0])}, ${rate(perGameHour(top.runs, p.ms))} a game hour, at a mean quality of ${Math.round(top.quality / top.runs)}.`;
    if (rows.length > 1) {
      const mean = rows.reduce((a, r) => a + r.quality, 0) / runs;
      line += ` ${plural(rows.length, "recipe", "recipes")} ran ${plural(runs, "craft", "crafts")} in all, at ${Math.round(mean)} on average.`;
    }
    h.textContent = line;
  }

  function productionStories() {
    const p = production();
    madeAndUsed(p);
    stock(p);
    workshops(p);
  }


  // Performance (D3.1): the tick and its phases, the loops beside it, the
  // queues, the sockets and the database

  // What each phase is, in words, and its colour, and what each loop is.
  // Positions, tax and subscriptions are both: the phase starts what the loop
  // does. One the page doesn't know is still drawn, by its own name
  const WORK = {
    lock: { say: "waiting for the game's lock", color: "--fl-deeds" },
    commands: { say: "players' commands", color: "--s3" },
    movement: { say: "movement", color: "--s2" },
    extraction: { say: "gathering", color: "--fl-deposits" },
    crafting: { say: "crafting", color: "--fl-fees" },
    regeneration: { say: "regrowing nodes", color: "--fl-refunds" },
    trades: { say: "expiring trade requests", color: "--fl-newcomers" },
    broadcast: { say: "broadcasting positions", color: "--s1" },
    writes: { say: "handing saves to the writer", color: "--fl-rent" },
    positions: { say: "reading positions to save", color: "--fl-tax" },
    tax: { say: "starting the property tax", color: "--fl-withdrawals" },
    subscriptions: { say: "starting subscriptions' lapse", color: "--fl-other" },
  };
  const LOOPS = {
    automation: { say: "stations' automation", color: "--fl-fees" },
    market: { say: "the market's matching", color: "--s3" },
    bazaar: { say: "the bazaar's sweep", color: "--fl-rent" },
    writer: { say: "the writer's saves of gathers and crafts, in four lanes", color: "--s2" },
    positions: { say: "the position save", color: "--fl-tax" },
    tax: { say: "the property tax", color: "--fl-withdrawals" },
    subscriptions: { say: "subscriptions' lapse", color: "--fl-other" },
    sampler: { say: "the Observatory's sampler", color: "--s1" },
  };
  // The bars each have a phase of their own up to here; the rest share one
  const PHASE_LINES = 6;

  let perf = { lines: [], key: "", rows: null, over: null };

  /** Events every reader of the bus missed over a sample's window */
  const missed = (l) => Object.values(l.events_lagged ?? {}).reduce((a, n) => a + n, 0);
  /** A percentile's bucket bound: null is past the last of them */
  const bound = (v) => (v == null || v === Infinity ? `over ${ms(1000)}` : ms(v));
  const say = (name, words = WORK) => words[name]?.say ?? item(name);

  /** The range's samples with a load, in bins: ticks, ticks over, and each phase's time */
  function binLoad(shown) {
    const binMs = BIN_MINUTES[state.hours] * 60_000;
    const first = Math.floor(rangeFrom() / binMs) * binMs;
    const bins = Array.from({ length: Math.ceil((Date.now() - first) / binMs) }, (_, k) => ({ at: first + k * binMs, ticks: 0, over: 0, secs: {} }));
    for (const s of shown) {
      const b = bins[Math.floor((s.at_ms - first) / binMs)];
      if (!b) continue;
      b.ticks += s.load.ticks;
      b.over += s.load.ticks_over ?? 0;
      for (const [name, p] of Object.entries(s.load.phases ?? {})) b.secs[name] = (b.secs[name] ?? 0) + p.secs;
    }
    return { binMs, bins };
  }

  /** Each phase or loop over the range: its runs, time, slow statements, and the worst sample's p95 */
  function workOver(shown, part) {
    const out = new Map();
    for (const s of shown) {
      for (const [name, p] of Object.entries(s.load[part] ?? {})) {
        const w = out.get(name) ?? { name, runs: 0, secs: 0, p95: null, slow: 0 };
        w.runs += p.runs;
        w.secs += p.secs;
        w.slow += p.slow_statements ?? 0;
        // A p95 past the last bucket is null though it ran: the worst of all
        if (p.runs > 0) w.p95 = p.p95_ms == null || w.p95 === Infinity ? Infinity : Math.max(w.p95 ?? 0, p.p95_ms);
        out.set(name, w);
      }
    }
    return [...out.values()];
  }

  async function performance() {
    if (state.covered.load > rangeFrom()) {
      $("tick-headline").textContent = "Reading the tick's history.";
      try { await readSeries("load"); } catch (e) { if (e instanceof SignedOut) return; console.warn(e); }
    }
    chart("tick");
    const shown = inRange((s) => s.load);
    if (!shown.length) {
      for (const id of ["tick", "phases", "over", "queues", "sockets", "db"]) $(`${id}-headline`).textContent = "Waiting for the first sample.";
      return;
    }
    const l = shown.at(-1).load;
    $("tick-headline").textContent = l.ticks === 0
      ? "No tick ran in the last ten seconds."
      : `The tick takes ${ms(l.tick_mean_ms)} on average, ${bound(l.tick_p95_ms)} at p95 and ${bound(l.tick_p99_ms)} at p99, of its ${ms(l.tick_period_ms)}.`;
    phases(shown);
    over(shown);
    for (const name of ["commands", "writer", "bus", "sockets", "pool", "txns"]) chart(name);
    queuesHeadline(shown);
    socketsHeadline(shown);
    dbHeadline(shown);
  }

  function phases(shown) {
    const { binMs, bins } = binLoad(shown);
    const ticks = shown.reduce((a, s) => a + s.load.ticks, 0);
    const work = workOver(shown, "phases").filter((w) => w.secs > 0).sort((a, b) => b.secs - a.secs);
    const tickSecs = work.reduce((a, w) => a + w.secs, 0);

    // The biggest phases each have a bar; the rest share one
    const own = work.slice(0, work.length > PHASE_LINES ? PHASE_LINES - 1 : PHASE_LINES);
    const rest = work.slice(own.length);
    const lines = [
      ...own.map((w) => ({ key: w.name, label: capital(w.name), color: WORK[w.name]?.color ?? "--fl-other", names: [w.name] })),
      ...(rest.length ? [{ key: "rest", label: "Everything else", color: "--fl-other", names: rest.map((w) => w.name) }] : []),
    ];
    // Each bar stacks up from zero, so each phase is drawn to its running
    // total, and the legend gives its own
    const values = {};
    const ownValues = {};
    for (const line of lines) { values[line.key] = []; ownValues[line.key] = []; }
    for (const b of bins) {
      let up = 0;
      for (const line of lines) {
        const v = b.ticks ? (line.names.reduce((a, n) => a + (b.secs[n] ?? 0), 0) * 1000) / b.ticks : null;
        ownValues[line.key].push(v);
        values[line.key].push(v ? (up += v) : null);
      }
    }
    const x = bins.map((b) => (b.at + binMs / 2) / 1000);
    const key = lines.map((line) => line.key).join();
    const rebuild = key !== perf.key;
    perf = {
      lines,
      key,
      rows: { x, shown, values, own: ownValues },
      over: { x, shown, values: { over: bins.map((b) => (b.ticks ? b.over || null : null)) }, own: { over: bins.map((b) => (b.ticks ? b.over : null)) } },
    };
    if (rebuild) state.charts.phases?.rebuild();
    chart("phases");

    const loops = workOver(shown, "loops").filter((w) => w.runs > 0).sort((a, b) => b.secs - a.secs);
    const rangeSecs = shown.reduce((a, s) => a + s.since_ms, 0) / 1000;
    workTable(work, tickSecs, loops, rangeSecs);

    const h = $("phases-headline");
    if (!ticks || !tickSecs) return void (h.textContent = `No tick ran ${since(shown[0])}.`);
    const top = work[0];
    let line = `${capital(since(shown[0]))} a tick took ${ms((tickSecs * 1000) / ticks)} on average, and ${say(top.name)} took the most of it: ${ms((top.secs * 1000) / ticks)}, ${Math.round((100 * top.secs) / tickSecs)}%.`;
    const busiest = loops[0];
    if (busiest && rangeSecs > 0) {
      const busy = busiest.secs / rangeSecs;
      line += ` Beside the tick, the busiest is ${say(busiest.name, LOOPS)}: ${busy < 0.01 ? "under 1%" : `${Math.round(busy * 100)}%`} of the time, at ${ms((busiest.secs * 1000) / busiest.runs)} a run.`;
    }
    const slow = [
      ...work.map((w) => ({ ...w, words: say(w.name) })),
      ...loops.map((w) => ({ ...w, words: say(w.name, LOOPS) })),
    ].filter((w) => w.slow > 0).sort((a, b) => b.slow - a.slow);
    if (slow.length) line += ` Slow statements: ${andList(slow.slice(0, 3).map((w) => `${nf.format(w.slow)} in ${w.words}`))}.`;
    h.textContent = line;
  }

  /** Every phase and loop: its runs, mean, worst p95, share and slow statements */
  function workTable(phases, tickSecs, loops, rangeSecs) {
    const group = (title, n) => {
      const tr = document.createElement("tr");
      tr.className = "group";
      const th = document.createElement("th");
      th.colSpan = 6;
      th.textContent = n ? title : `${title}: nothing ran`;
      tr.append(th);
      return tr;
    };
    const row = (w, whole, words) => {
      const tr = document.createElement("tr");
      const share = whole > 0 ? w.secs / whole : 0;
      const bar = document.createElement("span");
      bar.className = "share-bar";
      bar.style.width = `max(1px, ${Math.min(100, share * 100)}%)`;
      bar.style.background = `var(${words[w.name]?.color ?? "--fl-other"})`;
      tr.title = capital(say(w.name, words));
      tr.append(
        cell(capital(item(w.name))),
        cell(nf.format(w.runs), "num dim"),
        cell(ms(w.runs ? (w.secs * 1000) / w.runs : null), "num now"),
        cell(w.p95 == null ? "–" : bound(w.p95), "num dim"),
        cell([bar, share < 0.005 ? "<1%" : `${Math.round(share * 100)}%`], "num share"),
        cell(w.slow ? nf.format(w.slow) : "–", `num ${w.slow ? "down" : "dim"}`),
      );
      return tr;
    };
    $("work").tBodies[0].replaceChildren(
      group("In the tick", phases.length),
      ...phases.map((w) => row(w, tickSecs, WORK)),
      group("Beside it", loops.length),
      ...loops.map((w) => row(w, rangeSecs, LOOPS)),
    );
  }

  function over(shown) {
    chart("over");
    const ticks = shown.reduce((a, s) => a + s.load.ticks, 0);
    const n = shown.reduce((a, s) => a + (s.load.ticks_over ?? 0), 0);
    const period = shown.at(-1).load.tick_period_ms;
    const h = $("over-headline");
    if (!ticks) return void (h.textContent = `No tick ran ${since(shown[0])}.`);
    if (n === 0) return void (h.textContent = `None of ${plural(ticks, "tick", "ticks")} ran over its ${ms(period)} ${since(shown[0])}.`);
    const worst = shown.reduce((a, s) => ((s.load.ticks_over ?? 0) > (a.load.ticks_over ?? 0) ? s : a));
    const share = (100 * n) / ticks;
    h.textContent = `${capital(plural(n, "tick", "ticks"))} of ${nf.format(ticks)} ran over ${ms(period)} ${since(shown[0])}, ${share < 1 ? "under 1%" : `${Math.round(share)}%`}. The most in ten seconds was ${nf.format(worst.load.ticks_over)}, at ${clock(worst.at_ms)}.`;
  }

  const READERS = { sockets: "the sockets", admin_tap: "the Observatory's tap", admin_stream: "admin streams" };

  function queuesHeadline(shown) {
    const counted = shown.filter((s) => s.load.command_queue_p99 !== undefined);
    const ticks = counted.reduce((a, s) => a + s.load.ticks, 0);
    const commands = counted.reduce((a, s) => a + s.load.commands, 0);
    let line;
    if (!counted.length) line = "This server doesn't count its commands.";
    else if (!ticks) line = "No tick took up the queue.";
    else if (commands === 0) line = `No command came in ${since(counted[0])}.`;
    else {
      const deepest = counted.some((s) => s.load.ticks > 0 && s.load.command_queue_p99 == null)
        ? `over ${nf.format(1000)}`
        : nf.format(Math.max(...counted.map((s) => s.load.command_queue_p99 ?? 0)));
      line = `${capital(plural(commands, "command", "commands"))} came in ${since(counted[0])}, ${Number((commands / ticks).toFixed(2))} a tick, and at the 99th percentile no more than ${deepest} waited for one.`;
    }
    line += writerLine(shown);
    const lost = shown.reduce((a, s) => a + missed(s.load), 0);
    const resyncs = shown.reduce((a, s) => a + s.load.socket_resyncs, 0);
    const bus = latest((s) => s.events)?.events;
    if (lost === 0) {
      line += " No reader fell behind the event bus";
      line += bus ? `, and its backlog is ${plural(bus.queued, "event", "events")} of ${nf.format(bus.capacity)}.` : ".";
    } else {
      const by = {};
      for (const s of shown) for (const [r, n] of Object.entries(s.load.events_lagged ?? {})) by[r] = (by[r] ?? 0) + n;
      const who = Object.entries(by).filter(([, n]) => n > 0).sort((a, b) => b[1] - a[1]).map(([r, n]) => `${READERS[r] ?? item(r)} ${nf.format(n)}`);
      line += ` Readers missed ${plural(lost, "event", "events")} on the bus, ${andList(who)}`;
      line += resyncs ? `, and ${plural(resyncs, "socket", "sockets")} resynced.` : ".";
    }
    $("queues-headline").textContent = line;
  }

  /** The writer's backlog in words, with a space before it; none from a server before #407 */
  function writerLine(shown) {
    const counted = shown.filter((s) => s.load.writer_lag_ms !== undefined);
    if (!counted.length) return "";
    const now = counted.at(-1).load.writer_queued;
    const lags = counted.map((s) => s.load.writer_lag_ms).filter((v) => v != null);
    if (!lags.length) return ` The writer had nothing to save ${since(counted[0])}.`;
    const worst = Math.max(...lags);
    return ` The writer has ${now === 0 ? "nothing" : plural(now, "save", "saves")} waiting, and a tick's saves were written ${ms(lags.reduce((a, v) => a + v, 0) / lags.length)} after it on average, ${ms(worst)} in the worst ten seconds.`;
  }

  function socketsHeadline(shown) {
    const l = shown.at(-1).load;
    const opened = shown.reduce((a, s) => a + s.load.sockets_opened, 0);
    const dropped = shown.reduce((a, s) => a + s.load.sockets_dropped, 0);
    let line = `${capital(plural(l.sockets_open, "socket is", "sockets are"))} open, for ${plural(l.players_in_world, "avatar", "avatars")} in the world.`;
    line += ` ${capital(plural(opened, "socket", "sockets"))} opened ${since(shown[0])}`;
    line += dropped === 0 ? ", and none dropped." : `, and ${nf.format(dropped)} dropped, gone without a word or unwritable.`;
    $("sockets-headline").textContent = line;
  }

  function dbHeadline(shown) {
    const l = shown.at(-1).load;
    const used = l.db_pool_size - l.db_pool_idle;
    const peak = Math.max(...shown.map((s) => s.load.db_pool_size - s.load.db_pool_idle));
    const full = shown.filter((s) => s.load.db_pool_size > 0 && s.load.db_pool_idle === 0).length;
    const secs = shown.reduce((a, s) => a + s.since_ms / 1000, 0);
    const tps = shown.reduce((a, s) => a + s.load.db_transactions_per_sec * (s.since_ms / 1000), 0) / Math.max(1, secs);
    let line = `${capital(plural(used, "connection", "connections"))} of the pool's ${nf.format(l.db_pool_size)} ${used === 1 ? "is" : "are"} in use, and at most ${nf.format(peak)} ${since(shown[0])}`;
    line += full ? `; it was full in ${plural(full, "sample", "samples")}.` : ".";
    line += ` The database runs ${nf.format(Math.round(tps))} transactions a second on average, ${nf.format(Math.round(l.db_transactions_per_sec))} in the last ten seconds.`;
    $("db-headline").textContent = line;
  }

  // ---------------------------------------------------------------------------
  // The town (D4.2): who's on, the roster as the town's newest report has
  // it, how the questions were settled and what the layers cost. The roster
  // is `GET /admin/town`, read at most every TOWN_READ_MS while the tab is
  // open; the rest is each sample's `town`, the reports over its window.

  const TOWN_READ_MS = 15_000;
  // The roster rows shown before "Show all"
  const ROSTER_ROWS = 50;
  // The roster's columns
  const ROSTER_COLUMNS = 9;
  // C3's yardstick for a citizen-day: 100 decisions a game hour over a
  // 14-hour working day (the-capitol-citizens.md, C3.3)
  const C3_DAY_DECISIONS = 1400;
  // Signed in, or in a session and trying to be
  const ON = new Set(["starting", "working", "resting", "offline", "waiting"]);
  // The roster's standings in the order a sort puts them
  const STANDINGS = ["working", "resting", "starting", "waiting", "offline", "away", "retired", "stopped"];

  const townView = {
    latest: undefined, // TownLatest; null when no town has reported, undefined until read
    readAt: 0,
    made: null, // `/admin/citizens`, for a server no town has reported to
    // Those who are on first, by default
    sort: prefs.get("observatory.roster.sort") ?? "standing",
    desc: prefs.get("observatory.roster.desc") === "1",
    show: "all",
    // The whole roster, not the first ROSTER_ROWS
    every: false,
    find: "",
  };

  async function town(reload = false) {
    if (state.covered.town > rangeFrom()) {
      try { await readSeries("town"); } catch (e) { if (e instanceof SignedOut) return; console.warn(e); }
    }
    try { await readTown(reload); } catch (e) {
      if (e instanceof SignedOut) return;
      console.warn(e);
      if (townView.latest === undefined) $("town-headline").textContent = "The roster couldn't be read.";
    }
    townOn();
    rosterTable();
    callingsByStyle();
    rulings();
    if (books) cost();
  }

  /** The town's newest report, at most every TOWN_READ_MS; the town tab and a citizen's page share it */
  async function readTown(reload = false) {
    if (!reload && Date.now() - townView.readAt <= TOWN_READ_MS) return;
    townView.readAt = Date.now();
    books ??= await getJson("/admin/books");
    townView.latest = await getJson("/admin/town");
    if (!townView.latest) townView.made ??= await getJson("/admin/citizens");
  }

  /** A citizen's name, as a link to their page (D4.3) */
  function citizenLink(name) {
    const a = text("a", name);
    a.href = `#citizen/${encodeURIComponent(name)}`;
    a.className = "who";
    return a;
  }

  // --- Who's on

  CHARTS.townOn = () => lineChart($("chart-town-on"), {
    has: (s) => s.players,
    axis: count,
    legend: count,
    series: [
      { label: "Signed in", color: "--s1", width: 2, value: (s) => s.players.citizens },
      { label: "Working", color: "--s2", value: (s) => (s.town ? s.town.standing.working ?? 0 : null) },
      { label: "Resting", color: "--s3", value: (s) => (s.town ? s.town.standing.resting ?? 0 : null) },
    ],
  });

  function townOn() {
    chart("townOn");
    const t = townView.latest;
    const h = $("town-on-headline");
    if (t === undefined) return void (h.textContent = "Reading the town's report.");
    if (!t) return void (h.textContent = "No town has reported in to this server. scripts/town.sh up starts one.");
    const by = {};
    for (const c of t.report.citizens) by[c.standing] = (by[c.standing] ?? 0) + 1;
    const n = t.report.citizens.length;
    if (!t.up) return void (h.textContent = `The town is down. Its last report came ${ago(t.received_ms)}, with ${plural(n, "citizen", "citizens")} on the roster.`);
    const on = t.report.citizens.filter((c) => ON.has(c.standing)).length;
    const parts = STANDINGS.filter((k) => ON.has(k) && by[k]).map((k) => `${nf.format(by[k])} ${k}`);
    let line = on === 0
      ? `None of the town's ${plural(n, "citizen", "citizens")} is on.`
      : `${nf.format(on)} of the town's ${plural(n, "citizen", "citizens")} ${on === 1 ? "is" : "are"} on: ${andList(parts)}.`;
    if (by.away) line += ` ${nf.format(by.away)} ${by.away === 1 ? "is" : "are"} away until their next session.`;
    const list = inRange((s) => s.players);
    if (list.length > 1) {
      const peak = list.reduce((a, s) => (s.players.citizens > a.players.citizens ? s : a));
      if (peak.players.citizens > on) line += ` The most signed in at once ${since(list[0])} was ${nf.format(peak.players.citizens)}, at ${clock(peak.at_ms)}.`;
    }
    if (by.retired) line += ` ${plural(by.retired, "citizen has", "citizens have")} retired: the server refused ${by.retired === 1 ? "its key" : "their keys"}.`;
    h.textContent = line;
  }

  // --- The roster

  const ROSTER_SORT = {
    name: (c) => c.name.toLowerCase(),
    // By favourite, the all-rounders together, then by what else it does
    role: (c) => `${c.role} ${callingShort(c)}`,
    standing: (c) => STANDINGS.indexOf(c.standing),
    phase: (c) => c.phase ?? "~",
    strands: (c) => c.strands ?? -Infinity,
    activity: (c) => `${stintWords(c) ?? ""} ${c.activity}`.toLowerCase(),
    // The most single-minded first: its biggest activity's share of its
    // work (K0.1)
    time: (c) => -(timeShares(c.time).sorted[0]?.share ?? -1),
    // Furthest along the road in last, a claimed deed counting past the steps
    road: (c) => (c.road_in ? c.road_in.done + (c.road_in.grant === "spent" ? 0.5 : 0) : -1),
    // Sorted up, the newest failure comes first
    failure: (c) => -(c.failure?.at_ms ?? -Infinity),
  };

  const ROAD_CHAPTERS = {
    arrival: "Arrival",
    road_in: "The road in",
    hall: "The hall",
    people: "People",
    deeds_office: "The Deeds Office",
    settle_in: "Settle in",
  };

  /** Where a citizen is on the road in (#450), in a few words */
  function roadIn(r) {
    if (!r) return "–";
    const deed = r.grant === "spent" ? "deed claimed" : r.grant === "open" ? "deed earned" : null;
    if (r.done >= r.of) return deed ? `walked, ${deed}` : "walked";
    const at = `${ROAD_CHAPTERS[r.chapter] ?? r.chapter ?? "on the road"}, ${r.done} of ${r.of}`;
    return deed ? `${at}, ${deed}` : at;
  }
  const ROSTER_SHOW = {
    all: () => true,
    on: (c) => ON.has(c.standing),
    working: (c) => c.standing === "working",
    resting: (c) => c.standing === "resting",
    away: (c) => c.standing === "away",
    failed: (c) => c.failure != null,
  };

  function rosterTable() {
    const t = townView.latest;
    const body = $("roster").tBodies[0];
    const h = $("town-headline");
    $("roster-tools").hidden = !t;
    for (const th of $("roster").tHead.rows[0].cells) {
      th.setAttribute("aria-sort", th.dataset.sort === townView.sort ? (townView.desc ? "descending" : "ascending") : "none");
    }
    if (t === undefined) return;
    if (!t) {
      const made = townView.made ?? [];
      h.textContent = made.length
        ? `This server has made ${plural(made.length, "citizen", "citizens")}, but no town has reported in, so there's no more to say of them yet.`
        : "This server has made no citizens.";
      $("roster-dek").textContent = "Every citizen this server has made, oldest first. Their trades and days come with the town's first report.";
      return void body.replaceChildren(...(made.length ? made.map(madeRow) : [emptyRow(ROSTER_COLUMNS, "No citizens yet. scripts/town.sh up makes a town of them.")]));
    }
    $("roster-dek").textContent = `Every citizen on the town's roster, as its report of ${clockSecs(t.report.at_ms)} has them. Click a heading to sort, or a name for that citizen's page.`;
    const find = townView.find.trim().toLowerCase();
    const key = ROSTER_SORT[townView.sort] ?? ROSTER_SORT.name;
    const rows = t.report.citizens
      .filter(ROSTER_SHOW[townView.show])
      .filter((c) => !find || [c.name, callingShort(c), stintWords(c) ?? "", c.activity, c.failure?.why ?? ""].some((f) => f.toLowerCase().includes(find)))
      .sort((a, b) => {
        const [x, y] = [key(a), key(b)];
        const by = x < y ? -1 : x > y ? 1 : 0;
        return (townView.desc ? -by : by) || a.name.localeCompare(b.name);
      });
    const shown = townView.every ? rows : rows.slice(0, ROSTER_ROWS);
    timeLegend();
    body.replaceChildren(...(rows.length ? shown.map(rosterRow) : [emptyRow(ROSTER_COLUMNS, "No citizen on the roster matches.")]));
    if (shown.length < rows.length) body.append(moreRow(rows.length));
    const n = t.report.citizens.length;
    let line = rows.length === n
      ? `The roster holds ${plural(n, "citizen", "citizens")}.`
      : `Showing ${nf.format(rows.length)} of the roster's ${plural(n, "citizen", "citizens")}.`;
    const failed = t.report.citizens.filter((c) => c.failure && c.failure.at_ms > rangeFrom()).length;
    if (failed) line += ` For ${plural(failed, "of them", "of them")}, something went wrong in ${window_()}.`;
    const richest = t.report.citizens.filter((c) => c.strands != null).sort((a, b) => b.strands - a.strands)[0];
    if (richest) line += ` The richest is ${richest.name}, ${callingSays(richest)}, with ${plural(richest.strands, "Strand", "Strands")}.`;
    const roads = t.report.citizens.filter((c) => c.road_in);
    if (roads.length) {
      const deeds = roads.filter((c) => c.road_in.grant === "spent").length;
      const walking = roads.filter((c) => c.road_in.grant !== "spent").length;
      line += ` ${plural(deeds, "citizen has", "citizens have")} walked the road in to the deed it earns, and ${nf.format(walking)} ${walking === 1 ? "walks" : "walk"} it still.`;
    }
    h.textContent = line;
  }

  function rosterRow(c) {
    const tr = document.createElement("tr");
    const standing = text("span", c.standing);
    standing.className = `standing standing-${c.standing}`;
    const when = text("span", `${ago(c.failure?.at_ms ?? 0)}${c.restarts ? `, ${plural(c.restarts, "restart", "restarts")}` : ""}`);
    when.className = "dim";
    const failure = c.failure ? [c.failure.why, " ", when] : c.restarts ? plural(c.restarts, "restart", "restarts") : "–";
    // The job of the moment before the step, while it's at work; resting
    // or away, what it says of itself
    const stint = c.standing === "working" && stintWords(c);
    const doing = stint ? [Object.assign(text("span", `${capital(stint)}: `), { className: "stint" }), c.activity] : c.activity;
    const calling = cell(callingShort(c), "trade");
    if (c.calling?.favourite) calling.title = `${capital(c.calling.style)}: its calling gives its favourite ${percent(c.calling.focus)} of its time`;
    tr.append(
      cell(["◇ ", citizenLink(c.name)]),
      calling,
      cell(standing),
      cell(c.phase ?? "–", "dim"),
      cell(c.strands == null ? "–" : nf.format(c.strands), "num"),
      cell(doing),
      cell(timeCell(c), "time"),
      cell(roadIn(c.road_in), c.road_in ? "" : "dim"),
      cell(failure, c.failure ? "failure" : "dim"),
    );
    return tr;
  }

  /** The key to the roster's time bars: each activity's ink */
  function timeLegend() {
    const legend = $("roster-legend");
    if (legend.childElementCount) return;
    legend.append(text("span", "Time by activity, since the town started: "));
    for (const [k, name] of ACTIVITIES.filter(([k]) => k !== "free")) {
      const item = text("span", name);
      item.className = "key";
      item.style.setProperty("--c", `var(--a-${k})`);
      legend.append(item);
    }
  }

  /** The row under a cut roster, which shows the rest */
  function moreRow(n) {
    const tr = document.createElement("tr");
    tr.className = "more";
    const b = text("button", `Show all ${nf.format(n)}`);
    b.type = "button";
    b.className = "quiet";
    b.addEventListener("click", () => { townView.every = true; rosterTable(); });
    const td = cell([`The first ${nf.format(ROSTER_ROWS)} of ${nf.format(n)}. `, b]);
    td.colSpan = ROSTER_COLUMNS;
    tr.append(td);
    return tr;
  }

  /** A citizen the server made, before any town has reported */
  function madeRow(c) {
    const tr = document.createElement("tr");
    tr.append(
      cell(["◇ ", citizenLink(c.name)]),
      cell("–", "dim"),
      cell(c.key ? "never reported" : "no key, so it can't sign in", "dim"),
      cell("–", "dim"),
      cell("–", "num dim"),
      cell(c.key?.last_used_at ? `signed in ${ago(Date.parse(c.key.last_used_at))}` : "–", "dim"),
      cell("–", "dim"),
      cell("–", "dim"),
      cell("–", "dim"),
    );
    return tr;
  }

  // --- Callings by style (#558): each citizen's favourite's share of its
  // time against §10's band, from the roster's time by activity, which
  // counts from the town's start. The shares and bands are capitol-town's
  // `citizen::measures` (#553), so the counts match a balance's.

  // §10's bands: an all-rounder's is its biggest job's
  const STYLE_ROWS = [
    { style: "specialist", name: "Specialists", band: [0.6, 0.8] },
    { style: "leaner", name: "Leaners", band: [0.35, 0.55] },
    { style: "all-rounder", name: "All-rounders", band: [0, 0.35] },
  ];
  // The eight jobs and the seven making jobs, as time by activity keys them
  const JOB_KEYS = Object.values(JOBS).map((j) => j.activity);
  const MAKING_KEYS = JOB_KEYS.filter((k) => k !== "trading");
  // What a share is of: the making jobs (§10's measure since #555), the
  // eight jobs, or all the work; `keys` null for all of it
  const SHARE_OF = {
    making: { keys: MAKING_KEYS, says: "of the making jobs" },
    jobs: { keys: JOB_KEYS, says: "of the eight jobs" },
    work: { keys: null, says: "of all its work" },
  };
  // The wealth table's groups (#559): `roles` by calling, `styles` by style.
  // `lines` are the groups the chart over time draws, furthest from even first
  const wealthView = {
    by: prefs.get("observatory.wealth.by") === "styles" ? "styles" : "roles",
    lines: [],
  };
  const stylesView = {
    of: SHARE_OF[prefs.get("observatory.styles.of")] ? prefs.get("observatory.styles.of") : "making",
    rows: null, // each style's row, with its dots, for the chart
    width: 0, // the chart's width when it was drawn
    lit: null, // the name of the citizen pointed at
    left: "", // the note under the chart with no dot pointed at
  };

  const SVG = "http://www.w3.org/2000/svg";
  /** An SVG element with its attributes */
  function svgEl(tag, attrs = {}) {
    const el = document.createElementNS(SVG, tag);
    for (const [k, v] of Object.entries(attrs)) el.setAttribute(k, String(v));
    return el;
  }

  /**
   * One citizen's dot: its share of `of` (its favourite's, or an
   * all-rounder's biggest job's), whether that's in its band, its ink, its
   * trading's share of its work and the job that took the most of the rest.
   * `share` null with nothing to measure: no work yet, or a trading
   * favourite held to the making jobs.
   */
  function styleDot(c, of) {
    const k = c.calling;
    const row = STYLE_ROWS.find((r) => r.style === k.style);
    const time = c.time ?? {};
    const hours = (key) => time[key] ?? 0;
    const keys = SHARE_OF[of].keys;
    const work = Object.entries(time).reduce((n, [key, h]) => n + (key === "free" ? 0 : h), 0);
    const whole = keys ? keys.reduce((n, key) => n + hours(key), 0) : work;
    const fav = k.favourite ? JOBS[k.favourite]?.activity ?? null : null;
    // The jobs a share is counted over, the biggest first
    const jobs = (keys ?? JOB_KEYS).filter((key) => hours(key) > 0).sort((a, b) => hours(b) - hours(a));
    const at = row.style === "all-rounder" ? jobs[0] : fav;
    const share = whole > 0 && at && (keys ?? JOB_KEYS).includes(at) ? hours(at) / whole : null;
    const lo = row.style === "all-rounder" ? -Infinity : row.band[0];
    const next = jobs.find((key) => key !== at && key !== "trading");
    return {
      c,
      row,
      at,
      share,
      inBand: share == null ? null : share >= lo && share <= row.band[1],
      ink: at ?? "free",
      trading: work > 0 ? hours("trading") / work : null,
      next: next ? { key: next, share: hours(next) / whole } : null,
      why: !(work > 0) ? "no work yet" : share == null && fav === "trading" ? "a trading favourite" : share == null ? "no job yet" : null,
    };
  }

  function median(xs) {
    if (!xs.length) return null;
    const s = [...xs].sort((a, b) => a - b);
    const mid = Math.floor(s.length / 2);
    return s.length % 2 ? s[mid] : (s[mid - 1] + s[mid]) / 2;
  }

  /** A dot's citizen, in words, for the note under the chart */
  function dotSays(d) {
    const of = SHARE_OF[stylesView.of].says;
    const job = (key) => lc(ACTIVITY[key] ?? key);
    const band = d.inBand ? "in its band" : d.row.style === "all-rounder" || d.share > d.row.band[1] ? "over its band" : "under its band";
    const lead = d.row.style === "all-rounder"
      ? `its biggest job, ${job(d.at)}, takes ${percent(d.share)} ${of}, ${band}`
      : `${job(d.at)} takes ${percent(d.share)} ${of}, ${band}`;
    const trading = d.trading == null ? "" : d.trading > 0 ? `Trading took ${percent(d.trading)} of its work` : "It hasn't traded";
    const next = d.next ? `${d.row.style === "all-rounder" ? "the next job" : "of the rest"}, ${job(d.next.key)} the most, ${percent(d.next.share)}` : "no other job";
    return [`◇ `, Object.assign(text("span", d.c.name), { className: "who" }), `, ${callingSays(d.c)}: ${lead}. ${trading}${trading ? "; " : ""}${next}. Click for their page.`];
  }

  /** The note under the chart: the dot pointed at, kept by name across a redraw, or how many are shown */
  function stylesNote(d) {
    stylesView.lit = d?.c.name ?? null;
    const note = $("styles-note");
    if (d) return void note.replaceChildren(...dotSays(d));
    note.textContent = stylesView.left ?? "";
  }

  function callingsByStyle() {
    const t = townView.latest;
    const h = $("styles-headline");
    const box = $("styles");
    if (t === undefined) return void (h.textContent = "Reading the town's report.");
    if (!t) {
      box.replaceChildren();
      return void (h.textContent = "No town has reported in to this server, so there are no callings to show.");
    }
    const called = t.report.citizens.filter((c) => c.calling && STYLE_ROWS.some((r) => r.style === c.calling.style));
    if (!called.length) {
      box.replaceChildren();
      return void (h.textContent = "The town's report says no citizen's calling: it's from a town before callings (K3.3).");
    }
    const of = stylesView.of;
    const dots = called.map((c) => styleDot(c, of));
    const shown = dots.filter((d) => d.share != null);
    const rows = STYLE_ROWS.map((r) => {
      const these = shown.filter((d) => d.row === r);
      return { ...r, dots: these, inBand: these.filter((d) => d.inBand).length, median: median(these.map((d) => d.share)) };
    });

    // The headline: each style's in band and its median
    const said = rows.filter((r) => r.dots.length).map((r, i) => {
      const n = `${nf.format(r.inBand)} of ${plural(r.dots.length, r.name.toLowerCase().replace(/s$/, ""), r.name.toLowerCase())}`;
      const at = `${i ? "at" : "the median at"} ${percent(r.median)}`;
      return r.style === "all-rounder"
        ? `${n} keep${r.inBand === 1 ? "s" : ""} ${r.dots.length === 1 ? "its" : "their"} biggest job to 35% or under, ${at}`
        : `${n} ${r.inBand === 1 ? "is" : "are"} in ${r.dots.length === 1 ? "its" : "their"} band, ${at}`;
    });
    let line = said.length ? `${capital(SHARE_OF[of].says)}: ${said.join("; ")}.` : `No citizen has worked at a job yet, so there's no share ${SHARE_OF[of].says} to show.`;
    const out = dots.filter((d) => d.share == null);
    const why = {};
    for (const d of out) why[d.why] = (why[d.why] ?? 0) + 1;
    const left = [
      why["no work yet"] && `${plural(why["no work yet"], "citizen hasn't", "citizens haven't")} worked yet`,
      why["no job yet"] && `${plural(why["no job yet"], "citizen has", "citizens have")} worked at no job yet`,
      why["a trading favourite"] && `${plural(why["a trading favourite"], "trading favourite has", "trading favourites have")} no share of the making jobs`,
    ].filter(Boolean);
    if (left.length) line += ` ${capital(andList(left))}.`;
    h.textContent = line;
    stylesView.left = `${plural(shown.length, "citizen", "citizens")} shown, their time counted from the town's start, as of its report of ${clockSecs(t.report.at_ms)}. Point at a dot to see who it is.`;
    stylesView.rows = rows;
    drawStyles();
    stylesNote(shown.find((d) => d.c.name === stylesView.lit) ?? null);
  }

  /** The chart: a row a style, its band behind it, its citizens as dots stacked where they crowd */
  function drawStyles() {
    const box = $("styles");
    const rows = stylesView.rows;
    const W = box.clientWidth;
    if (!rows || W <= 0) return;
    stylesView.width = W;
    const narrow = W < 520;
    const most = Math.max(...rows.map((r) => r.dots.length));
    const r = most > 150 ? 3 : most > 60 ? 4 : narrow ? 5 : 6;
    const padX = narrow ? 10 : 16;
    const x = (s) => padX + s * (W - 2 * padX);
    const step = r + 0.5;
    // How far a crowd stacks from a row's middle, before its dots overlap
    const MOST_HALF = 70;
    const LABEL = 24;
    const AXIS = 22;

    // Each row's dots stacked up and down from its middle where they'd touch
    const laid = rows.map((row) => {
      const placed = [];
      for (const d of [...row.dots].sort((a, b) => a.share - b.share || a.c.name.localeCompare(b.c.name))) {
        const cx = x(d.share);
        // The nearest free place to the middle, or where it overlaps least
        let best = { dy: 0, gap: -1 };
        for (let k = 0; Math.ceil(k / 2) * step <= MOST_HALF; k++) {
          const dy = (k % 2 ? 1 : -1) * Math.ceil(k / 2) * step;
          const gap = Math.min(Infinity, ...placed.map((p) => Math.hypot(p.cx - cx, p.dy - dy)));
          if (gap > best.gap) best = { dy, gap };
          if (gap >= 2 * r + 1) break;
        }
        placed.push({ d, cx, dy: best.dy });
      }
      const half = Math.max(16, ...placed.map((p) => Math.abs(p.dy) + r + 6));
      return { row, placed, half };
    });

    const H = laid.reduce((n, l) => n + LABEL + 2 * l.half + 10, 0) + AXIS;
    const svg = svgEl("svg", { viewBox: `0 0 ${W} ${H}`, height: H, role: "group", "aria-label": `Each citizen's favourite's share ${SHARE_OF[stylesView.of].says}, by style, against §10's bands` });
    const grid = svgEl("g");
    const ticks = [0, 0.2, 0.4, 0.6, 0.8, 1];
    for (const s of ticks) {
      grid.append(svgEl("line", { class: "grid", x1: x(s), x2: x(s), y1: LABEL - 4, y2: H - AXIS }));
      const t = svgEl("text", { class: "tick", x: x(s), y: H - 6, "text-anchor": s === 0 ? "start" : s === 1 ? "end" : "middle" });
      t.textContent = percent(s);
      grid.append(t);
    }
    svg.append(grid);

    let y = 0;
    for (const { row, placed, half } of laid) {
      const top = y + LABEL;
      const mid = top + half;
      const bottom = top + 2 * half;
      const name = svgEl("text", { class: "row-name", x: padX, y: y + 16 });
      name.textContent = row.name;
      const count = svgEl("tspan", { class: "row-count", dx: 8 });
      count.textContent = row.dots.length ? `${nf.format(row.inBand)} of ${nf.format(row.dots.length)} in band` : "none to show";
      name.append(count);
      svg.append(name);
      if (row.median != null) {
        const m = svgEl("text", { class: "row-count", x: W - padX, y: y + 16, "text-anchor": "end" });
        m.textContent = `median ${percent(row.median)}`;
        svg.append(m);
      }
      svg.append(svgEl("rect", { class: "track", x: x(0), y: top, width: x(1) - x(0), height: 2 * half, rx: 3 }));
      const [lo, hi] = row.band;
      svg.append(svgEl("rect", { class: "band", x: x(lo), y: top, width: x(hi) - x(lo), height: 2 * half }));
      for (const edge of row.style === "all-rounder" ? [hi] : [lo, hi]) {
        svg.append(svgEl("line", { class: "band-edge", x1: x(edge), x2: x(edge), y1: top, y2: bottom }));
      }
      if (row.median != null) {
        const m = svgEl("line", { class: "median", x1: x(row.median), x2: x(row.median), y1: top - 3, y2: bottom + 3 });
        m.append(Object.assign(svgEl("title"), { textContent: `The median, ${percent(row.median)}` }));
        svg.append(m);
      }
      for (const p of placed) {
        const d = p.d;
        const a = svgEl("a", { href: `#citizen/${encodeURIComponent(d.c.name)}`, class: `dot ${d.inBand ? "in" : "out"}${d.c.name === stylesView.lit ? " lit" : ""}` });
        const circle = svgEl("circle", { cx: p.cx.toFixed(1), cy: (mid + p.dy).toFixed(1), r });
        circle.style.setProperty("--c", `var(--a-${d.ink})`);
        a.append(circle);
        a.setAttribute("aria-label", `${d.c.name}: ${percent(d.share)}, ${d.inBand ? "in" : "out of"} band`);
        const lit = () => { a.classList.add("lit"); stylesNote(d); };
        const unlit = () => { a.classList.remove("lit"); stylesNote(null); };
        a.addEventListener("pointerenter", lit);
        a.addEventListener("pointerleave", unlit);
        a.addEventListener("focus", lit);
        a.addEventListener("blur", unlit);
        svg.append(a);
      }
      y = bottom + 10;
    }
    box.replaceChildren(svg);
  }

  function stylesControls() {
    const group = $("styles-of");
    for (const b of group.querySelectorAll("button")) b.setAttribute("aria-pressed", String(b.dataset.of === stylesView.of));
    group.addEventListener("click", (e) => {
      const b = e.target.closest("button[data-of]");
      if (!b) return;
      stylesView.of = b.dataset.of;
      prefs.set("observatory.styles.of", stylesView.of);
      for (const x of group.querySelectorAll("button")) x.setAttribute("aria-pressed", String(x === b));
      callingsByStyle();
    });
    new ResizeObserver(() => {
      const w = $("styles").clientWidth;
      if (w > 0 && w !== stylesView.width) drawStyles();
    }).observe($("styles"));
  }

  function rosterControls() {
    for (const th of $("roster").tHead.rows[0].cells) {
      th.querySelector("button").addEventListener("click", () => {
        const k = th.dataset.sort;
        // Strands sort richest first; the rest A to Z
        townView.desc = townView.sort === k ? !townView.desc : k === "strands";
        townView.sort = k;
        prefs.set("observatory.roster.sort", k);
        prefs.set("observatory.roster.desc", townView.desc ? "1" : "0");
        rosterTable();
      });
    }
    $("roster-filters").addEventListener("click", (e) => {
      const b = e.target.closest("button[data-show]");
      if (!b) return;
      townView.show = b.dataset.show;
      townView.every = false;
      for (const x of $("roster-filters").querySelectorAll("button")) x.setAttribute("aria-pressed", String(x === b));
      rosterTable();
    });
    $("roster-find").addEventListener("input", (e) => {
      townView.find = e.target.value;
      townView.every = false;
      rosterTable();
    });
  }

  // --- How they decide

  /** A ruling's colour: the layers by name, the fallbacks after */
  function rulingColor(label) {
    if (label === "rules") return "--s1";
    if (label === "jev") return "--s2";
    if (label.endsWith(", unsure")) return "--s3";
    if (label === "the rules' own pick") return "--s4";
    if (label === "carry on") return "--fl-other";
    return "--s5";
  }
  const layerName = (l) => (l === "rules" ? "the rules" : l === "jev" ? "Jev" : l);
  const possessive = (s) => (s.endsWith("s") ? `${s}'` : `${s}'s`);
  /** How a ruling reads in a sentence: "by the rules", "by Jev's pick, unsure" */
  function rulingSays(label) {
    if (label === "carry on") return "by carrying on";
    if (label === "the rules' own pick") return "by the rules' own pick";
    if (label.startsWith("waiting on ")) return `waiting on ${layerName(label.slice(11))}`;
    if (label.endsWith(", unsure")) return `by ${possessive(layerName(label.slice(0, -8)))} pick, unsure`;
    return `by ${layerName(label)}`;
  }

  /** The range's town summaries in the money's bins: rulings, cost and citizen-days; and the layers over the range */
  function binTown() {
    const binMs = BIN_MINUTES[state.hours] * 60_000;
    const first = Math.floor(rangeFrom() / binMs) * binMs;
    const bins = Array.from({ length: Math.ceil((Date.now() - first) / binMs) }, (_, k) => ({
      at: first + k * binMs, ms: 0, rulings: {}, decisions: 0, cents: 0, days: 0,
    }));
    const shown = inRange((s) => s.town);
    const layers = {};
    for (const s of shown) {
      // As `LayerReport::add` adds them
      for (const [name, l] of Object.entries(s.town.layers)) {
        const t = (layers[name] ??= { asked: 0, outcomes: {}, micros: 0, max_micros: 0, cents: 0 });
        t.asked += l.asked;
        for (const [o, n] of Object.entries(l.outcomes)) t.outcomes[o] = (t.outcomes[o] ?? 0) + n;
        t.micros += l.micros;
        t.max_micros = Math.max(t.max_micros, l.max_micros);
        t.cents += l.cents;
      }
      const b = bins[Math.floor((s.at_ms - first) / binMs)];
      if (!b) continue;
      b.ms += s.since_ms;
      for (const [by, n] of Object.entries(s.town.rulings)) {
        b.rulings[by] = (b.rulings[by] ?? 0) + n;
        b.decisions += n;
      }
      for (const l of Object.values(s.town.layers)) b.cents += l.cents;
      if (s.players && books) b.days += (s.players.citizens * s.since_ms) / books.day_ms;
    }
    return { binMs, bins, shown, layers };
  }

  let ruled = { lines: [], rows: null, key: "" };
  CHARTS.rulings = () => lineChart($("chart-rulings"), {
    rows: () => ruled.rows,
    axis: short,
    legend: count,
    series: () => ruled.lines.map((l) => ({ key: l.key, label: l.label, color: l.color, bars: true })).reverse(),
  });

  function rulings() {
    const p = binTown();
    const totals = {};
    for (const b of p.bins) for (const [by, n] of Object.entries(b.rulings)) totals[by] = (totals[by] ?? 0) + n;
    const lines = Object.keys(totals).filter((k) => totals[k]).sort((a, b) => totals[b] - totals[a])
      .map((key) => ({ key, label: capital(key), color: rulingColor(key) }));
    // Stacked up from zero, the most common at the bottom
    const values = {};
    const own = {};
    for (const l of lines) {
      values[l.key] = [];
      own[l.key] = [];
    }
    for (const b of p.bins) {
      let up = 0;
      for (const l of lines) {
        const v = b.rulings[l.key] ?? 0;
        own[l.key].push(b.ms ? v : null);
        values[l.key].push(!b.ms || v === 0 ? null : (up += v));
      }
    }
    const x = p.bins.map((b) => (b.at + p.binMs / 2) / 1000);
    const key = lines.map((l) => l.key).join();
    const rebuild = key !== ruled.key;
    ruled = { lines, key, rows: { x, shown: p.shown, values, own } };
    if (rebuild) state.charts.rulings?.rebuild();
    chart("rulings");

    const all = Object.values(totals).reduce((a, n) => a + n, 0);
    const h = $("rulings-headline");
    if (!p.shown.length) h.textContent = townView.latest ? "No sample in the range has heard from the town." : "Waiting for the town to report in.";
    else if (!all) h.textContent = `No citizen settled a question ${since(p.shown[0])}.`;
    else {
      const pct = (n) => { const v = (100 * n) / all; return v >= 1 ? `${Math.round(v)}%` : "under 1%"; };
      const parts = lines.slice(0, 4).map((l) => `${pct(totals[l.key])} ${rulingSays(l.key)}`);
      if (lines.length > 4) parts.push(`${pct(lines.slice(4).reduce((a, l) => a + totals[l.key], 0))} otherwise`);
      h.textContent = `Of ${plural(all, "question", "questions")} settled ${since(p.shown[0])}, ${andList(parts)}.`;
    }
    layerTable(p.layers);
  }

  function layerTable(layers) {
    const names = Object.keys(layers).sort((a, b) => layers[b].asked - layers[a].asked);
    const rows = names.map((name) => {
      const l = layers[name];
      const settled = l.outcomes.settled ?? 0;
      const other = Object.entries(l.outcomes).filter(([o, n]) => o !== "settled" && n).sort((a, b) => b[1] - a[1]).map(([o, n]) => `${nf.format(n)} ${o}`);
      const tr = document.createElement("tr");
      tr.append(
        cell(capital(layerName(name))),
        cell(nf.format(l.asked), "num"),
        cell(l.asked ? `${nf.format(settled)} (${Math.round((100 * settled) / l.asked)}%)` : "–", "num"),
        cell(other.length ? other.join(", ") : "–", other.length ? "" : "dim"),
        cell(l.asked ? ms(l.micros / l.asked / 1000) : "–", "num"),
        cell(l.asked ? ms(l.max_micros / 1000) : "–", "num"),
        cell(cents(l.cents), l.cents ? "num" : "num dim"),
      );
      return tr;
    });
    $("town-layers").tBodies[0].replaceChildren(...(rows.length ? rows : [emptyRow(7, "No layer was asked in the range.")]));
  }

  // --- What it costs

  /** US cents, to a sensible precision */
  function cents(v) {
    if (!v) return "0¢";
    if (v >= 100) return `$${(v / 100).toFixed(2)}`;
    return `${Number(v.toPrecision(2))}¢`;
  }

  let costRows = null;
  CHARTS.cost = () => lineChart($("chart-cost"), {
    rows: () => costRows,
    height: 180,
    axis: (v) => (v == null ? "" : cents(v)),
    legend: cents,
    series: [
      { key: "day", label: "A citizen-day on", color: "--s2", width: 2 },
      { key: "c3", label: "C3's day of 1,400", color: "--s4", width: 1.5, dash: [5, 4] },
    ],
  });

  function cost() {
    const p = binTown();
    costRows = {
      x: p.bins.map((b) => (b.at + p.binMs / 2) / 1000),
      shown: p.shown,
      values: {
        day: p.bins.map((b) => (b.ms && b.days > 0 ? b.cents / b.days : null)),
        c3: p.bins.map((b) => (b.ms && b.decisions > 0 ? (b.cents / b.decisions) * C3_DAY_DECISIONS : null)),
      },
    };
    chart("cost");

    const h = $("cost-headline");
    if (!p.shown.length) return void (h.textContent = townView.latest ? "No sample in the range has heard from the town." : "Waiting for the town to report in.");
    const spent = p.bins.reduce((a, b) => a + b.cents, 0);
    const days = p.bins.reduce((a, b) => a + b.days, 0);
    const decisions = p.bins.reduce((a, b) => a + b.decisions, 0);
    if (!decisions) return void (h.textContent = `No citizen settled a question ${since(p.shown[0])}, so nothing was spent.`);
    if (!spent) {
      const paid = Object.keys(p.layers).filter((l) => l !== "rules");
      return void (h.textContent = paid.length
        ? `The layers cost nothing ${since(p.shown[0])}: ${andList(paid.map(layerName))} ${paid.length === 1 ? "was" : "were"} asked, but spent nothing.`
        : `The town's ${plural(decisions, "decision", "decisions")} cost nothing ${since(p.shown[0])}: only the rules were asked, and they're free.`);
    }
    const c3 = cents((spent / decisions) * C3_DAY_DECISIONS);
    let line = `The layers spent ${cents(spent)} ${since(p.shown[0])}`;
    line += days > 0
      ? `: ${cents(spent / days)} for each citizen signed in for a game day, or ${c3} on C3's day of 1,400 decisions.`
      : `, or ${c3} on C3's day of 1,400 decisions.`;
    const t = townView.latest;
    const total = Object.values(t?.report.layers ?? {}).reduce((a, l) => a + l.cents_total, 0);
    if (t?.up && total > 0) line += ` This run of the town has spent ${cents(total)} since ${clock(t.report.started_ms)}.`;
    h.textContent = line;
  }

  // ---------------------------------------------------------------------------
  // One citizen (D4.3), at `#citizen/<name>`: who they are and what they're
  // doing, from the town's report; their Strands over the range and their
  // newest moves, from `GET /admin/player`; their last decisions, from the
  // report the town tab holds; and a way to their dot on the map.

  const CITIZEN_READ_MS = 10_000;
  // How close the map comes in to follow someone from their page: this many
  // world units across its shorter side
  const FOLLOW_SPAN = 1500;

  const citizenView = {
    name: null,
    detail: undefined, // PlayerDetail; null when nobody is called `name`, undefined until read
    readAt: 0,
    hours: null,
    rows: null,
  };
  // The chart before the books are read: it's built as the page opens
  const NO_STRANDS = { x: [], values: { strands: [] }, shown: [], moments: true };
  citizenView.rows = NO_STRANDS;

  // Each kind of move as the moves table names it, and as a sentence says it
  const MOVES = {
    exchange_deposit: ["Deposit", "deposits at the Exchange"],
    exchange_withdraw: ["Withdrawal", "changing Strands back at the Exchange"],
    deed_purchase: ["Deed", "deeds"],
    deed_refund: ["Deed refund", "deed refunds"],
    foreclosure_refund: ["Foreclosure refund", "foreclosure refunds"],
    tax_payment: ["Tax", "tax"],
    property_tax: ["Property tax", "property tax"],
    admin_adjustment: ["Grant", "admins' grants"],
    market_escrow: ["Buy order", "buy orders' escrow"],
    market_refund: ["Escrow back", "escrow back from buy orders"],
    market_sale: ["Sale", "selling on the market"],
    market_fee: ["Market fee", "the market's fees"],
    bazaar_rent: ["Stall rent", "stall rent"],
    bazaar_purchase: ["Bazaar buy", "buying at the bazaar"],
    bazaar_sale: ["Bazaar sale", "selling at the bazaar"],
    bazaar_refund: ["Bazaar refund", "the bazaar's refunds"],
    trade_sent: ["Trade, paid", "paying in trades"],
    trade_received: ["Trade, paid in", "being paid in trades"],
  };
  const moveLabel = (kind) => MOVES[kind]?.[0] ?? capital(kind.replace(/_/g, " "));
  const moveSays = (kind) => MOVES[kind]?.[1] ?? kind.replace(/_/g, " ");

  // How each standing reads after a citizen's name
  const STANDING_SAYS = {
    starting: "is starting up",
    working: "is working",
    resting: "is resting",
    offline: "has lost the server and is trying to get back on",
    waiting: "is waiting to get back on",
    away: "is away",
    retired: "has retired: the server refused its key",
    stopped: "has stopped",
  };

  /** The town's row for the citizen called `name`, if the newest report has one */
  const rowOf = (name) => townView.latest?.report.citizens.find((c) => c.name.toLowerCase() === name.toLowerCase()) ?? null;

  async function citizen() {
    const v = citizenView;
    const name = v.name;
    const fresh = v.detail !== undefined && v.hours === state.hours && Date.now() - v.readAt < CITIZEN_READ_MS;
    try {
      await readTown();
      if (!fresh) {
        v.readAt = Date.now();
        const res = await api(`/admin/player?name=${encodeURIComponent(name)}&hours=${state.hours}`);
        if (res.status !== 404 && !res.ok) throw new Error(`/admin/player: ${res.status}`);
        const detail = res.status === 404 ? null : await res.json();
        // Moved on to someone else meanwhile
        if (v.name !== name) return;
        v.detail = detail;
        v.hours = state.hours;
      }
    } catch (e) {
      if (e instanceof SignedOut) return;
      console.warn(e);
      if (v.detail === undefined) $("citizen-headline").textContent = `${name} couldn't be read.`;
    }
    if (state.tab !== "citizen" || v.name !== name) return;
    citizenHead();
    citizenStrands();
    citizenMoves();
    citizenDecisions();
  }

  /** On to the citizen called `name`: nothing of the last one shows meanwhile */
  function openCitizen(name) {
    const v = citizenView;
    if (v.name?.toLowerCase() === name.toLowerCase()) return;
    Object.assign(v, { name, detail: undefined, readAt: 0, rows: NO_STRANDS });
    $("citizen-name").replaceChildren(text("span", "◇"), ` ${name}`);
    $("citizen-headline").textContent = `Reading ${name}.`;
    for (const id of ["citizen-dek", "citizen-where"]) $(id).textContent = "";
    $("citizen-follow").hidden = true;
    $("citizen-tiles").replaceChildren();
    $("citizen-moves").tBodies[0].replaceChildren();
    $("citizen-decisions").replaceChildren();
    $("citizen-strands-headline").textContent = "Reading the books.";
    $("citizen-decisions-headline").textContent = "Reading the town's report.";
    document.title = `${name} · The Observatory`;
    // From far down the roster, start at the top of the page
    window.scrollTo(0, 0);
  }

  // --- Who they are

  function citizenHead() {
    const v = citizenView;
    const d = v.detail;
    const c = rowOf(v.name);
    const name = d?.name ?? c?.name ?? v.name;
    const isCitizen = d ? d.citizen : c != null;
    $("citizen-name").replaceChildren(...(isCitizen ? [text("span", "◇"), " "] : []), name);
    const h = $("citizen-headline");
    if (d === null && !c) {
      h.textContent = `Nobody on this server is called ${v.name}.`;
      $("citizen-dek").textContent = "Names are matched whatever their case. A citizen made on another server, the usual world or the town, isn't here.";
      return void $("citizen-tiles").replaceChildren();
    }

    // The headline: who, and what they're at
    if (c) {
      let line = `${name}, ${callingSays(c)}, ${STANDING_SAYS[c.standing] ?? `is ${c.standing}`}`;
      // The job of the moment, while it's at work (Callings K3.3)
      const stint = stintWords(c);
      if (stint && c.standing === "working") line += `, ${stintFor(c.doing.hours)} into ${stint}`;
      // "is away" and "away until 19:00" read as one
      const at = c.activity.trim();
      line += !at ? "." : at.toLowerCase().startsWith(c.standing) ? `${at.slice(c.standing.length).replace(/^[:,]?\s*/, " ").trimEnd()}.` : `: ${at}.`;
      if (stint && c.standing !== "working") line += ` It was last ${stint}.`;
      if (townView.latest && !townView.latest.up) line += ` The town is down, so that's as its last report had it, ${ago(townView.latest.received_ms)}.`;
      h.textContent = line;
    } else {
      h.textContent = isCitizen
        ? `${name} is a citizen, but not on the roster of the town reporting to this server.`
        : `${name} is a person, not a citizen: nobody's report speaks for them.`;
    }

    // The dek: their calling against their time, their day, their troubles
    const parts = [];
    const k = c?.calling;
    const t = timeShares(c?.time);
    if (k?.favourite) {
      const fav = jobName(k.favourite).toLowerCase();
      const had = t.work.find((a) => a.key === JOBS[k.favourite]?.activity);
      parts.push(`Its calling gives ${fav} ${percent(k.focus)} of its time${t.total ? `, and so far ${fav} has had ${percent(had?.share ?? 0)} of its work` : ""}.`);
    } else if (k) {
      parts.push("Its calling spreads its time over every job, none much above the rest.");
    }
    if (t.total) {
      const top = t.sorted.slice(0, 4).map((a) => `${lc(a.name)} ${percent(a.share)}`);
      const rest = t.sorted.length > 4 ? ", and the rest on more" : "";
      parts.push(`Since the town started its work has gone on ${andList(top)}${rest}${t.free != null ? `, and ${percent(t.free)} of its time was free` : ""}.`);
    }
    if (c?.phase) parts.push(`It's the ${c.phase} part of ${possessive(name)} day.`);
    if (c?.failure) parts.push(`The last thing that went wrong, ${ago(c.failure.at_ms)}: ${c.failure.why}`);
    if (c?.restarts) parts.push(`${capital(plural(c.restarts, "restart", "restarts"))} since the town started.`);
    $("citizen-dek").textContent = parts.join(" ");

    // The figures
    const tiles = [];
    const tile = (label, value, unit, sub, level = "") => {
      const el = document.createElement("div");
      el.className = "tile";
      el.dataset.level = level;
      const l = text("p", label);
      l.className = "label";
      const val = text("p", value);
      val.className = "value";
      if (unit) val.append(text("small", ` ${unit}`));
      const s = text("p", sub);
      s.className = "sub";
      el.append(l, val, s);
      tiles.push(el);
    };
    if (d) {
      const change = d.strands - d.opening;
      tile("In hand", nf.format(d.strands), "Strands", c?.strands != null && c.strands !== d.strands ? `The town's last report said ${nf.format(c.strands)}.` : "As the server's books have it.");
      tile("In buy orders", nf.format(d.escrow), "Strands", d.escrow ? "Held in escrow until the orders fill or are cancelled." : "No buy order holds any.");
      tile(capital(window_()), signed(change), "Strands", d.moves ? `From ${nf.format(d.opening)} to ${nf.format(d.strands)}.` : "Nothing moved.", change > 0 ? "good" : change < 0 ? "bad" : "");
      tile("Moves", nf.format(d.moves), "", d.recent.length ? `The newest ${ago(d.recent[0].at_ms)}.` : "None ever.");
    } else if (c?.strands != null) {
      tile("In hand", nf.format(c.strands), "Strands", "As the town's last report had it.");
    }
    $("citizen-tiles").replaceChildren(...tiles);

    // Where they are
    const w = d?.walker;
    const where = $("citizen-where");
    if (w) {
      const zone = zoneAt(w.x, w.y);
      where.textContent = `${name} is out in the world${zone ? `, in the ${zone}` : ""}, at ${nf.format(Math.round(w.x))}, ${nf.format(Math.round(w.y))}: ${doing(w)}.`;
    } else {
      where.textContent = d ? `${name} isn't in the world now.` : "";
    }
    $("citizen-follow").hidden = !w;
  }

  /** The zone at a point, if the map has read the zones' rings */
  function zoneAt(x, y) {
    const t = map.terrain;
    const zones = map.layers?.zones;
    if (!t || !zones) return null;
    const r = Math.hypot(x - t.cx, y - t.cy);
    const z = zones.find((z) => r >= z.inner && r < z.outer);
    return z ? `${capital(z.zone.replace(/_/g, " "))} zone` : null;
  }

  // --- Their Strands

  CHARTS.citizenStrands = () => lineChart($("chart-citizen-strands"), {
    rows: () => citizenView.rows,
    axis: short,
    legend: (v) => `${count(v)} Strands`,
    series: [{ key: "strands", label: "In hand", color: "--s2", width: 2, step: true }],
  });

  function citizenStrands() {
    const d = citizenView.detail;
    const h = $("citizen-strands-headline");
    if (!d) {
      citizenView.rows = NO_STRANDS;
      chart("citizenStrands");
      h.textContent = d === null ? "There are no books to read." : "Reading the books.";
      return;
    }
    // From the range's start, through each move, to now
    const now = Date.now();
    const x = [d.from_ms / 1000, ...d.balance.map((p) => p.at_ms / 1000), now / 1000];
    const strands = [d.opening, ...d.balance.map((p) => p.strands), d.strands];
    citizenView.rows = { x, values: { strands }, shown: [], moments: true };
    chart("citizenStrands");

    if (!d.moves) {
      h.textContent = `${d.name} held ${plural(d.strands, "Strand", "Strands")} all through ${window_()}: none moved.`;
      return;
    }
    const change = d.strands - d.opening;
    let line = change === 0
      ? `${possessive(d.name)} Strands came back to ${nf.format(d.strands)} over ${window_()}, after ${plural(d.moves, "move", "moves")}`
      : `${possessive(d.name)} Strands went ${change > 0 ? "up" : "down"} ${nf.format(Math.abs(change))} over ${window_()}, from ${nf.format(d.opening)} to ${nf.format(d.strands)}, in ${plural(d.moves, "move", "moves")}`;
    const flows = Object.entries(d.flows).filter(([, v]) => v);
    const most = (sign) => flows.filter(([, v]) => Math.sign(v) === sign).sort((a, b) => Math.abs(b[1]) - Math.abs(a[1]))[0];
    const [into, out] = [most(1), most(-1)];
    const why = [
      into && `the most in from ${moveSays(into[0])} (${signed(into[1])})`,
      out && `the most out to ${moveSays(out[0])} (${signed(out[1])})`,
    ].filter(Boolean);
    h.textContent = why.length ? `${line}: ${andList(why)}.` : `${line}.`;
  }

  function citizenMoves() {
    const d = citizenView.detail;
    const body = $("citizen-moves").tBodies[0];
    if (!d) return void body.replaceChildren();
    if (!d.recent.length) return void body.replaceChildren(emptyRow(5, `${d.name} has never moved a Strand.`));
    const today = new Date().toDateString();
    body.replaceChildren(...d.recent.map((m) => {
      const tr = document.createElement("tr");
      const when = text("time", new Date(m.at_ms).toDateString() === today ? clockSecs(m.at_ms) : `${day(m.at_ms)}, ${clock(m.at_ms)}`);
      when.dateTime = new Date(m.at_ms).toISOString();
      tr.append(
        cell(when, "dim"),
        cell(moveLabel(m.kind)),
        cell(signed(m.amount), `num ${m.amount > 0 ? "up" : m.amount < 0 ? "down" : ""}`),
        cell(nf.format(m.strands), "num"),
        cell(m.description ?? "–", m.description ? "" : "dim"),
      );
      return tr;
    }));
  }

  // --- Their last decisions

  // How a layer's turn came out, as a colour: it settled, it passed it up, it failed
  const TURN_TONE = { settled: "up", unsure: "pass", escalated: "pass", pending: "pass", "off menu": "down", slow: "down", broke: "down", late: "down" };

  function citizenDecisions() {
    const v = citizenView;
    const c = rowOf(v.name);
    const name = c?.name ?? v.detail?.name ?? v.name;
    const h = $("citizen-decisions-headline");
    const list = $("citizen-decisions");
    if (townView.latest === undefined) return void (h.textContent = "Reading the town's report.");
    if (!townView.latest) {
      list.replaceChildren();
      return void (h.textContent = "No town has reported in to this server, so there are no decisions to show.");
    }
    if (!c) {
      list.replaceChildren();
      return void (h.textContent = v.detail?.citizen === false
        ? `${name} is a person, and makes up their own mind.`
        : `${name} isn't on the town's roster, so the town sends no decisions for them.`);
    }
    const ds = [...c.decisions].reverse();
    if (!ds.length) {
      list.replaceChildren();
      return void (h.textContent = `${name} hasn't settled a question since this server started.`);
    }
    const newest = ds[0];
    let line = `${possessive(name)} newest decision, ${ago(newest.at_ms)}, was ${newest.picked ? `“${newest.picked}”` : "nothing, from an empty menu"}, ${rulingSays(newest.by)}.`;
    if (ds.length > 1) {
      const by = {};
      for (const d of ds) by[d.by] = (by[d.by] ?? 0) + 1;
      const kinds = Object.entries(by).sort((a, b) => b[1] - a[1]);
      line += kinds.length === 1
        ? ` So ${ds.length === 2 ? "was the one" : `were the ${nf.format(ds.length - 1)}`} before it.`
        : ` Of the last ${nf.format(ds.length)}, ${andList(kinds.map(([k, n]) => `${nf.format(n)} ${n === 1 ? "was" : "were"} settled ${rulingSays(k)}`))}.`;
    }
    h.textContent = line;
    list.replaceChildren(...ds.map(decisionItem));
  }

  function decisionItem(d) {
    const li = document.createElement("li");
    li.className = "decision";

    const head = document.createElement("p");
    head.className = "decision-head";
    const when = text("time", clockSecs(d.at_ms));
    when.dateTime = new Date(d.at_ms).toISOString();
    const q = text("span", capital(d.question));
    q.className = "question";
    const by = text("span", rulingSays(d.by));
    by.className = "by";
    by.style.setProperty("--c", `var(${rulingColor(d.by)})`);
    head.append(when, q, by);

    // The menu, with what stood marked
    const menu = document.createElement("ul");
    menu.className = "menu";
    menu.setAttribute("aria-label", "The menu");
    for (const entry of d.menu) {
      const item_ = text("li", entry);
      if (entry === d.picked) {
        item_.className = "picked";
        const terms = [d.quantity != null && plural(d.quantity, "unit", "units"), d.price != null && `at ${plural(d.price, "Strand", "Strands")}`].filter(Boolean);
        if (terms.length) item_.append(text("small", ` ${terms.join(" ")}`));
      }
      menu.append(item_);
    }
    if (!d.menu.length) menu.append(Object.assign(text("li", "An empty menu"), { className: "dim" }));

    // Each layer's turn, in the order they were asked
    const trail = document.createElement("ol");
    trail.className = "trail";
    trail.setAttribute("aria-label", "How each layer answered");
    for (const t of d.trail) {
      const step = document.createElement("li");
      const layer = text("span", capital(layerName(t.layer)));
      layer.className = "layer";
      const outcome = text("span", t.outcome);
      outcome.className = "outcome";
      outcome.dataset.tone = TURN_TONE[t.outcome] ?? "";
      step.append(layer, " ", outcome, text("small", ` ${ms(t.micros / 1000)}`));
      trail.append(step);
    }
    li.append(head, menu, ...(d.trail.length ? [trail] : []));
    return li;
  }

  // ---------------------------------------------------------------------------
  // Tuning (D6.2): every knob beside the chart it's watched on, set or reset
  // live, and the lot exported and imported as a TOML preset

  const TUNE_CHARTS = ["sinks", "production", "distribution", "prices", "tick"];
  // The line a knob moves on its chart, for the swatch beside it. A family's
  // knobs go by the family's key
  const KNOB_LINE = {
    exchange_fee_rate: "--fl-withdrawals",
    listing_fee_percent: "--fl-fees",
    stall_rent_per_day: "--fl-rent",
    property_tax_rate: "--fl-tax",
    watch_tax_per_hour: "--fl-tax",
    deed_price: "--fl-deeds",
    yield: "--uf-gathered",
    node_regen_rate: "--uf-gathered",
    watch_gini: "--budget",
    watch_off_target: "--marker",
    watch_tick_p95: "--budget",
    weather_wear_claimed: "--uf-repaired",
    weather_wear_public: "--uf-repaired",
    outdoor_tool_wear: "--uf-worn_out",
    floor_permit_scale: "--fl-deeds",
  };
  // The knobs as the server last said, and each one's row by key
  let knobs = null;
  const knobRows = new Map();

  // A fraction is typed as the percent it reads as
  const typed = (k, v) => (k.unit === "fraction" ? Number((v * 100).toFixed(6)) : v);
  const untyped = (k, v) => (k.unit === "fraction" ? v / 100 : v);
  const AFFIX = { fraction: ["", "%"], percent: ["", "%"], strands: ["", "Strands"], multiplier: ["×", ""], ratio: ["", ""] };
  const knobOf = (key) => knobs?.find((k) => k.key === key);

  async function loadKnobs() {
    let list;
    try { ({ knobs: list } = await getJson("/admin/tunables")); } catch (e) {
      if (!(e instanceof SignedOut)) $("tuning-headline").textContent = "The knobs couldn't be read.";
      return;
    }
    showKnobs(list);
  }

  function showKnobs(list) {
    knobs = list;
    state.giniWatch = knobOf("watch_gini")?.value ?? state.giniWatch;
    state.charts.spread?.update();
    state.charts["tune-distribution"]?.update();
    const off = knobs.filter((k) => k.value !== k.default);
    $("tuning-headline").textContent = off.length === 0
      ? `Every one of the ${knobs.length} knobs is at its default.`
      : off.length <= 3
        ? `${off.length} of ${knobs.length} knobs ${off.length === 1 ? "is" : "are"} off ${off.length === 1 ? "its default" : "their defaults"}: ${off.map((k) => `${k.label} at ${unit(k.unit, k.value)}`).join(", ")}.`
        : `${off.length} of ${knobs.length} knobs are off their defaults.`;

    // The rows are made once for a set of knobs, then kept, so a value being
    // typed survives a change that comes in from elsewhere
    const keys = knobs.map((k) => k.key).join();
    if (keys !== [...knobRows.keys()].join()) {
      knobRows.clear();
      const groups = new Map();
      for (const k of knobs) {
        const g = TUNE_CHARTS.includes(k.chart) ? k.chart : "other";
        if (!groups.has(g)) groups.set(g, []);
        groups.get(g).push(knobRow(k));
      }
      for (const story of document.querySelectorAll(".tune")) {
        const rows = groups.get(story.dataset.chart) ?? [];
        story.querySelector("tbody").replaceChildren(...rows.map((r) => r.tr));
        if (story.dataset.chart === "other") story.hidden = rows.length === 0;
      }
    }
    for (const k of knobs) paintKnob(k);
    $("reset-all").disabled = off.length === 0;
  }

  function knobRow(k) {
    const tr = document.createElement("tr");
    const input = document.createElement("input");
    input.type = "number";
    input.inputMode = "decimal";
    input.className = "knob-input";
    input.min = typed(k, k.min);
    input.max = typed(k, k.max);
    input.step = typed(k, k.step);
    input.title = `${unit(k.unit, k.min)} to ${unit(k.unit, k.max)}`;
    input.setAttribute("aria-label", k.label);
    input.addEventListener("input", () => input.setCustomValidity(""));
    input.addEventListener("keydown", (e) => {
      if (e.key === "Escape") { input.value = typed(k, knobOf(k.key).value); input.blur(); }
    });
    input.addEventListener("change", () => {
      const v = Number(input.value);
      if (input.value === "" || !Number.isFinite(v)) return void (input.value = typed(k, knobOf(k.key).value));
      setKnobs({ [k.key]: untyped(k, v) }, input);
    });
    const reset = document.createElement("button");
    reset.type = "button";
    reset.className = "quiet";
    reset.textContent = "Reset";
    reset.setAttribute("aria-label", `Reset ${k.label} to ${unit(k.unit, k.default)}`);
    reset.addEventListener("click", () => setKnobs({ [k.key]: k.default }, input));

    const swatch = document.createElement("i");
    swatch.className = "swatch";
    swatch.style.setProperty("--c", `var(${KNOB_LINE[k.key.split(".")[0]] ?? "--rule"})`);
    swatch.setAttribute("aria-hidden", "true");
    const [before, after] = AFFIX[k.unit] ?? ["", ""];
    const now = document.createElement("label");
    now.className = "knob-now";
    now.append(text("span", before), input, text("span", after));
    const name = text("span", k.label.replace(/_/g, " "));
    name.className = "knob-name";
    const sub = document.createElement("span");
    sub.className = "knob-sub";
    sub.append(text("code", k.key), " ", text("span", `· ${unit(k.unit, k.default)} by default`));
    // A multiplier over a price says the price, and what it comes to (#431)
    const base = k.base_strands == null ? null : text("span", "");
    if (base) sub.append(" ", base);
    tr.append(
      cell([swatch, name, sub]),
      cell(now, "num now"),
      cell(reset, "num"),
    );
    const row = { tr, input, reset, base };
    knobRows.set(k.key, row);
    return row;
  }

  function paintKnob(k) {
    const row = knobRows.get(k.key);
    if (!row) return;
    const off = k.value !== k.default;
    row.tr.classList.toggle("off", off);
    row.reset.disabled = !off;
    if (document.activeElement !== row.input) row.input.value = typed(k, k.value);
    if (row.base) {
      const now = k.value === 1 ? k.base_strands : Math.max(1, Math.round(k.base_strands * k.value));
      row.base.textContent = `· times ${nf.format(k.base_strands)} Strands a deed` + (now === k.base_strands ? "" : `, now ${nf.format(now)}`);
    }
  }

  /** The words of a refusal: the server's `error`, or the status */
  async function refusal(res) {
    const body = await res.text();
    try { return JSON.parse(body).error ?? body; } catch { return body || `The server said ${res.status}.`; }
  }

  /** Set knobs by key. A refusal is said at `input`, and the knob put back */
  async function setKnobs(values, input) {
    let res;
    try {
      res = await api("/admin/tunables", { method: "PUT", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ values }) });
    } catch (e) {
      if (e instanceof SignedOut) return;
      res = null;
    }
    if (!res?.ok) {
      const why = res ? await refusal(res) : "The server didn't answer, so nothing changed.";
      for (const key of Object.keys(values)) paintKnob(knobOf(key));
      if (input) { input.setCustomValidity(why); input.reportValidity(); } else presetNote(why);
      return false;
    }
    showKnobs((await res.json()).knobs);
    return true;
  }

  function presetNote(s) { $("preset-note").textContent = s; }

  /** What moved between two lists of the knobs, as "Property tax a week to 5%" */
  function moved(before, after) {
    const was = new Map(before.map((k) => [k.key, k.value]));
    return after.filter((k) => was.get(k.key) !== k.value).map((k) => `${k.label} to ${unit(k.unit, k.value)}`);
  }

  async function exportPreset() {
    let res;
    try { res = await api("/admin/tunables/preset"); } catch (e) { if (!(e instanceof SignedOut)) presetNote("The server didn't answer."); return; }
    if (!res.ok) return presetNote(await refusal(res));
    const url = URL.createObjectURL(new Blob([await res.text()], { type: "application/toml" }));
    const a = document.createElement("a");
    a.href = url;
    a.download = `tunables-${new Date().toISOString().slice(0, 16).replace(/[-:T]/g, "")}.toml`;
    document.body.append(a);
    a.click();
    a.remove();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
    const off = (knobs ?? []).filter((k) => k.value !== k.default).length;
    presetNote(`Saved ${a.download}: every one of the ${knobs?.length ?? ""} knobs, ${off === 0 ? "all at their defaults" : `${off} off ${off === 1 ? "its default" : "their defaults"}`}.`);
  }

  async function importPreset(file) {
    const before = knobs ?? [];
    let res;
    try {
      res = await api("/admin/tunables/preset", { method: "PUT", headers: { "Content-Type": "application/toml" }, body: await file.text() });
    } catch (e) { if (!(e instanceof SignedOut)) presetNote("The server didn't answer, so nothing changed."); return; }
    if (!res.ok) return presetNote(`${file.name} wasn't taken, and nothing changed. ${await refusal(res)}`);
    const { knobs: after } = await res.json();
    showKnobs(after);
    const changes = moved(before, after);
    presetNote(changes.length === 0
      ? `Imported ${file.name}: it matched every knob, so nothing changed.`
      : `Imported ${file.name}: ${changes.length <= 4 ? changes.join(", ") : plural(changes.length, "knob", "knobs") + " changed"}.`);
  }

  // Resetting every knob takes a second click within a few seconds, rather
  // than a dialog
  let resetArmed = null;
  async function resetAll() {
    const b = $("reset-all");
    const off = (knobs ?? []).filter((k) => k.value !== k.default);
    if (!off.length) return;
    if (!resetArmed) {
      b.textContent = `Reset all ${off.length}? Click again`;
      resetArmed = setTimeout(() => { resetArmed = null; b.textContent = "Reset every knob"; }, 4000);
      return;
    }
    clearTimeout(resetArmed);
    resetArmed = null;
    b.textContent = "Reset every knob";
    const before = knobs;
    if (await setKnobs(Object.fromEntries(off.map((k) => [k.key, k.default])))) {
      presetNote(`Reset ${plural(moved(before, knobs).length, "knob", "knobs")} to ${off.length === 1 ? "its default" : "their defaults"}.`);
    }
  }

  async function tuning() {
    if (!knobs) await loadKnobs();
    const from = rangeFrom();
    // The tick's chart draws from the base field set (#411)
    if (!books || state.covered.money > from || state.covered.making > from) {
      try {
        books ??= await getJson("/admin/books");
        await Promise.all([readSeries("money"), readSeries("making")]);
      } catch (e) { if (e instanceof SignedOut) return; console.warn(e); }
    }
    // The economy's own sums, which its tab's stories write too
    if (books) {
      flows();
      madeAndUsed(production());
    }
    distribution();
    await prices();
    for (const c of TUNE_CHARTS) chart(`tune-${c}`);
    tuneHeadlines();
  }

  /** Each line's total over the range, biggest first, as [line, amount] */
  function totalsOf(set, side, sign) {
    const own = set.rows?.own ?? {};
    return set.lines
      .filter((l) => l.side === side)
      .map((l) => [l, sign * (own[l.key] ?? []).reduce((a, v) => a + (v ?? 0), 0)])
      .filter(([, n]) => n > 0)
      .sort((a, b) => b[1] - a[1]);
  }

  function tuneHeadlines() {
    const first = (rows) => rows?.shown?.[0];
    const list = (parts) => parts.length < 2 ? parts.join("") : `${parts.slice(0, -1).join(", ")} and ${parts.at(-1)}`;

    const sinks = totalsOf(money, "out", -1);
    const s0 = first(money.rows);
    $("tune-sinks-headline").textContent = !s0
      ? "Waiting for the first sample."
      : sinks.length === 0
        ? `Nothing was taken out of the world ${since(s0)}.`
        : `${capital(sinks[0][0].say)} took ${nf.format(sinks[0][1])} Strands ${since(s0)}${sinks.length > 1 ? `; ${list(sinks.slice(1).map(([l, n]) => `${l.say} ${nf.format(n)}`))}` : ""}.`
          + (sinks.some(([l]) => l.key === "tax") ? "" : " No tax was paid.");

    const made = totalsOf(making, "in", 1);
    const m0 = first(making.rows);
    $("tune-production-headline").textContent = !m0
      ? "Waiting for the first sample."
      : made.length === 0
        ? `Nothing came into the world ${since(m0)}.`
        : `${capital(list(made.map(([l, n]) => `${plural(n, "unit", "units")} ${l.say(n)}`)))} ${since(m0)}.`;

    const d = latest((s) => s.distribution?.gini != null)?.distribution;
    $("tune-distribution-headline").textContent = !d
      ? "Waiting for a sample that says."
      : state.giniWatch == null
        ? `The Gini is ${d.gini.toFixed(2)}.`
        : `The Gini is ${d.gini.toFixed(2)}, ${d.gini > state.giniWatch ? "over" : "under"} the watch's ${state.giniWatch.toFixed(2)}.`;

    const firing = (latest((s) => s.watches)?.watches ?? []).filter((w) => w.kind === "off_target").map((w) => item(w.item));
    const off = knobOf("watch_off_target");
    $("tune-prices-headline").textContent = firing.length
      ? `${capital(list(firing))} ${firing.length === 1 ? "has" : "have"} been off target by ${off ? unit(off.unit, off.value) : "the watch's share"} or more for a game day.`
      : `No item has been off target by ${off ? unit(off.unit, off.value) : "the watch's share"} for a game day.`;

    const l = latest((s) => s.load?.ticks > 0)?.load;
    const watch = knobOf("watch_tick_p95");
    $("tune-tick-headline").textContent = !l
      ? "Waiting for the first sample."
      : `The tick's p95 is ${ms(l.tick_p95_ms)} against a budget of ${ms(l.tick_period_ms)}${watch ? `; the watch fires over ${ms(watch.value * l.tick_period_ms)}` : ""}.`;
  }

  function tuningControls() {
    $("preset-export").addEventListener("click", exportPreset);
    $("preset-import").addEventListener("click", () => $("preset-file").click());
    $("preset-file").addEventListener("change", (e) => {
      const file = e.target.files?.[0];
      e.target.value = "";
      if (file) importPreset(file);
    });
    $("reset-all").addEventListener("click", resetAll);
  }

  // ---------------------------------------------------------------------------
  // The map (D1.3): `/terrain` turned once into paths in world units, the
  // walkers from `/admin/positions` every two seconds, eased between reads,
  // and the slower layers from `/admin/map` and `/world/nodes`. It takes over
  // from the React client's F1 debug camera for watching the world. The wilds are
// drawn flat: the height field is Rust only (#415).

  const POSITIONS_MS = 2_000;
  const MAP_LAYERS_MS = 30_000;
  const NODES_MS = 60_000;
  const LAYERS = ["people", "nodes", "plots", "stations", "heat"];
  const NODE_TYPES = ["grass", "reed_bed", "tree", "fallen_branches", "old_growth", "rock_outcrop", "clay_bank", "copper_vein", "tin_vein", "iron_vein"];
  const stillness = matchMedia("(prefers-reduced-motion: reduce)");

  const map = {
    terrain: null,
    loading: null,
    layers: null,
    nodes: null,
    // By player id: the walker, where it's drawn from and where it's going
    walkers: new Map(),
    movedAt: 0,
    view: null,
    on: new Set(savedLayers()),
    timers: [],
    pending: null,
    hover: null,
    follow: null,
    // A player to follow when the map next opens (D4.3)
    want: null,
    frame: 0,
    palette: null,
    size: { w: 0, h: 0 },
  };

  function savedLayers() {
    try {
      const saved = JSON.parse(prefs.get("observatory.layers"));
      if (Array.isArray(saved)) return saved.filter((l) => LAYERS.includes(l));
    } catch { /* the default */ }
    return ["people"];
  }

  /** The map's inks, read once a theme from its tokens */
  function palette() {
    if (map.palette) return map.palette;
    const t = (n) => token(`--m-${n}`);
    const nodes = {};
    for (const n of NODE_TYPES) nodes[n] = token(`--n-${n}`);
    map.palette = {
      outside: token("--paper"), zoneA: t("zone-a"), zoneB: t("zone-b"), edge: t("edge"),
      water: t("water"), stream: t("stream"), park: t("park"), lot: t("lot"),
      road: t("road"), arterial: t("arterial"), trail: t("trail"), landmark: t("landmark"),
      ownedCitizen: t("owned-citizen"), ownedPerson: t("owned-person"),
      citizen: t("citizen"), person: t("person"), halo: t("halo"), path: t("path"),
      heat: t("heat"), heatMode: t("heat-mode") || "multiply",
      station: t("station"), label: t("label"), ink: token("--ink"), dim: token("--ink-dim"),
      nodes, otherNode: token("--n-other"),
    };
    return map.palette;
  }

  function buildTerrain(t) {
    const path = (pts, closed = false) => {
      const p = new Path2D();
      pts.forEach(([x, y], i) => (i ? p.lineTo(x, y) : p.moveTo(x, y)));
      if (closed) p.closePath();
      return p;
    };
    const rings = (list) => {
      const p = new Path2D();
      for (const ring of list) p.addPath(path(ring, true));
      return p;
    };
    // One path per kind and width of road, so a draw is a handful of strokes
    const roads = new Map();
    for (const r of t.roads) {
      const key = `${r.road_type}:${r.width}`;
      if (!roads.has(key)) roads.set(key, { type: r.road_type, width: r.width, path: new Path2D() });
      roads.get(key).path.addPath(path(r.points));
    }
    const order = { trail: 0, suburban: 1, grid: 2, arterial: 3 };
    const lots = new Map();
    const lotOutlines = new Path2D();
    for (const lot of t.terrain_plots) {
      const p = path(lot.corners, true);
      lots.set(lot.id, p);
      lotOutlines.addPath(p);
    }
    const course = (c) => ({
      path: path(c.points.map((p) => [p.x, p.y])),
      width: c.points.reduce((s, p) => s + p.width, 0) / Math.max(1, c.points.length),
    });
    const landmarks = new Map(t.landmarks.map((l) => [l.id, l]));
    const buildings = new Path2D();
    for (const l of t.landmarks) if (l.kind !== "square") buildings.addPath(path(l.corners, true));
    return {
      size: t.world_size,
      cx: t.center_x,
      cy: t.center_y,
      roads: [...roads.values()].sort((a, b) => order[a.type] - order[b.type] || a.width - b.width),
      lots,
      lotOutlines,
      lotCount: t.terrain_plots.length,
      parks: rings(t.parks.flatMap((p) => p.rings)),
      lakes: rings(t.lakes.flatMap((l) => l.rings)),
      rivers: t.rivers.map(course),
      streams: t.streams.map(course),
      waterRings: t.water_rings,
      landmarks,
      buildings,
    };
  }

  // ---- the view: the world point at the canvas's centre, and px a unit

  function fitView() {
    const { w, h } = map.size;
    const t = map.terrain;
    map.view = { cx: t.cx, cy: t.cy, scale: (Math.min(w, h) / t.size) * 0.98 };
    map.follow = null;
  }
  const toScreen = (x, y) => {
    const v = map.view;
    return [(x - v.cx) * v.scale + map.size.w / 2, (y - v.cy) * v.scale + map.size.h / 2];
  };
  const toWorld = (sx, sy) => {
    const v = map.view;
    return [(sx - map.size.w / 2) / v.scale + v.cx, (sy - map.size.h / 2) / v.scale + v.cy];
  };
  function zoomAt(sx, sy, factor) {
    const t = map.terrain;
    const min = (Math.min(map.size.w, map.size.h) / t.size) * 0.6;
    const scale = Math.min(8, Math.max(min, map.view.scale * factor));
    const [wx, wy] = toWorld(sx, sy);
    map.view.scale = scale;
    // Keep the point under the cursor where it was, unless following someone
    if (!map.follow) {
      map.view.cx = wx - (sx - map.size.w / 2) / scale;
      map.view.cy = wy - (sy - map.size.h / 2) / scale;
    }
    drawSoon();
  }

  // ---- the walkers, eased from where they were drawn to where they are

  function easing() {
    if (stillness.matches) return 1;
    return Math.min(1, (window.performance.now() - map.movedAt) / POSITIONS_MS);
  }
  function walkerAt(m, k = easing()) {
    return [m.fromX + (m.w.x - m.fromX) * k, m.fromY + (m.w.y - m.fromY) * k];
  }
  function takePositions(list) {
    const k = easing();
    const next = new Map();
    for (const w of list) {
      const had = map.walkers.get(w.player_id);
      const [fx, fy] = had ? walkerAt(had, k) : [w.x, w.y];
      // A jump (a teleport, a sign-in far off) isn't eased across the city
      const far = Math.hypot(w.x - fx, w.y - fy) > 400;
      next.set(w.player_id, { w, fromX: far ? w.x : fx, fromY: far ? w.y : fy });
    }
    map.walkers = next;
    map.movedAt = window.performance.now();
    if (map.follow && !next.has(map.follow)) map.follow = null;
    mapHeadline();
    drawSoon();
  }

  // ---- drawing

  function drawSoon() {
    if (map.frame || state.tab !== "map") return;
    map.frame = requestAnimationFrame(() => {
      map.frame = 0;
      drawMap();
      // Keep going while anyone is still easing to where they are
      if (easing() < 1) drawSoon();
    });
  }

  function drawMap() {
    const canvas = $("map-canvas");
    const t = map.terrain;
    if (!t || !map.view || !map.size.w) return;
    const ctx = canvas.getContext("2d");
    const p = palette();
    const dpr = devicePixelRatio || 1;
    const v = map.view;
    if (map.follow) {
      const m = map.walkers.get(map.follow);
      if (m) [v.cx, v.cy] = walkerAt(m);
    }
    const px = 1 / v.scale;

    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.fillStyle = p.outside;
    ctx.fillRect(0, 0, map.size.w, map.size.h);

    // In world units from here
    ctx.setTransform(dpr * v.scale, 0, 0, dpr * v.scale, dpr * (map.size.w / 2 - v.cx * v.scale), dpr * (map.size.h / 2 - v.cy * v.scale));
    const disc = (r) => { ctx.beginPath(); ctx.arc(t.cx, t.cy, r, 0, Math.PI * 2); };
    const zones = map.layers?.zones ?? [{ zone: "world", inner: 0, outer: t.size / 2 }];
    for (let i = zones.length - 1; i >= 0; i--) {
      disc(zones[i].outer);
      ctx.fillStyle = i % 2 ? p.zoneB : p.zoneA;
      ctx.fill();
    }
    disc(t.size / 2);
    ctx.strokeStyle = p.edge;
    ctx.lineWidth = 1.5 * px;
    ctx.stroke();

    ctx.fillStyle = p.park;
    ctx.fill(t.parks, "evenodd");
    ctx.fillStyle = p.water;
    ctx.fill(t.lakes, "evenodd");
    ctx.lineCap = ctx.lineJoin = "round";
    ctx.strokeStyle = p.stream;
    for (const s of t.streams) { ctx.lineWidth = Math.max(s.width, 0.75 * px); ctx.stroke(s.path); }
    ctx.strokeStyle = p.water;
    for (const r of t.rivers) { ctx.lineWidth = Math.max(r.width, 1.25 * px); ctx.stroke(r.path); }
    // The moat and canals, broken where a bridge crosses
    ctx.lineCap = "butt";
    for (const ring of t.waterRings) {
      const gaps = ring.bridges
        .map((b) => [(b.angle_degrees * Math.PI) / 180, b.width / ring.radius])
        .sort((a, b) => a[0] - b[0]);
      ctx.lineWidth = Math.max(ring.width, 1.5 * px);
      if (!gaps.length) { disc(ring.radius); ctx.stroke(); continue; }
      gaps.forEach(([a, half], i) => {
        const [b, nextHalf] = gaps[(i + 1) % gaps.length];
        ctx.beginPath();
        ctx.arc(t.cx, t.cy, ring.radius, a + half, (i + 1 === gaps.length ? b + Math.PI * 2 : b) - nextHalf);
        ctx.stroke();
      });
    }

    // Lots once they're big enough to tell apart
    if (v.scale > 0.05) {
      ctx.strokeStyle = p.lot;
      ctx.lineWidth = px;
      ctx.stroke(t.lotOutlines);
    }
    if (map.on.has("plots") && map.layers) {
      for (const plot of map.layers.plots) {
        const lot = t.lots.get(plot.terrain_id);
        if (!lot) continue;
        ctx.fillStyle = plot.citizen ? p.ownedCitizen : p.ownedPerson;
        ctx.fill(lot);
      }
    }

    ctx.lineCap = "round";
    for (const r of t.roads) {
      ctx.strokeStyle = r.type === "arterial" ? p.arterial : r.type === "trail" ? p.trail : p.road;
      ctx.lineWidth = Math.max(r.width, (r.type === "arterial" ? 1.5 : r.type === "trail" ? 0.75 : 1) * px);
      ctx.setLineDash(r.type === "trail" ? [6 * px, 4 * px] : []);
      ctx.stroke(r.path);
    }
    ctx.setLineDash([]);
    ctx.fillStyle = p.landmark;
    ctx.fill(t.buildings);

    // Screen space from here: marks keep their size at any zoom
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    const inView = (sx, sy, pad = 20) => sx > -pad && sy > -pad && sx < map.size.w + pad && sy < map.size.h + pad;

    if (v.scale < 0.12 && map.layers) zoneLabels(ctx, p, zones);
    if (map.on.has("heat") && map.layers) heat(ctx, p);

    if (map.on.has("nodes") && map.nodes) {
      const r = Math.max(1.6, Math.min(4, 5 * v.scale));
      for (const n of map.nodes) {
        const [sx, sy] = toScreen(n.x, n.y);
        if (!inView(sx, sy)) continue;
        ctx.globalAlpha = n.state === "depleted" ? 0.35 : 1;
        ctx.fillStyle = p.nodes[n.resource_type] || p.otherNode;
        ctx.beginPath();
        ctx.arc(sx, sy, r, 0, Math.PI * 2);
        ctx.fill();
      }
      ctx.globalAlpha = 1;
    }

    if (map.on.has("stations") && map.layers) {
      const s = Math.max(3, Math.min(7, 9 * v.scale));
      ctx.lineWidth = 1.25;
      for (const st of map.layers.stations) {
        const [sx, sy] = toScreen(st.x, st.y);
        if (!inView(sx, sy)) continue;
        ctx.fillStyle = p.station;
        ctx.fillRect(sx - s / 2, sy - s / 2, s, s);
        if (st.citizen) { ctx.strokeStyle = p.citizen; ctx.strokeRect(sx - s / 2 - 1.5, sy - s / 2 - 1.5, s + 3, s + 3); }
      }
    }

    if (map.on.has("people")) people(ctx, p, inView);
    scaleBar(ctx, p);
  }

  function people(ctx, p, inView) {
    const k = easing();
    // Where each is walking to, faint, once there's room to see it
    if (map.view.scale > 0.04) {
      ctx.strokeStyle = p.path;
      ctx.lineWidth = 1;
      ctx.setLineDash([2, 3]);
      ctx.beginPath();
      for (const m of map.walkers.values()) {
        if (!m.w.dest) continue;
        const [sx, sy] = toScreen(...walkerAt(m, k));
        const [dx, dy] = toScreen(...m.w.dest);
        if (!inView(sx, sy) && !inView(dx, dy)) continue;
        ctx.moveTo(sx, sy);
        ctx.lineTo(dx, dy);
      }
      ctx.stroke();
      ctx.setLineDash([]);
    }
    const marked = [];
    for (const m of map.walkers.values()) {
      const [sx, sy] = toScreen(...walkerAt(m, k));
      if (!inView(sx, sy)) continue;
      if (m.w.player_id === map.follow || m.w.player_id === map.hover?.id) marked.push([m, sx, sy]);
      // A citizen is a hollow diamond, its mark everywhere; a person a dot
      ctx.lineWidth = 3;
      ctx.strokeStyle = p.halo;
      ctx.beginPath();
      if (m.w.citizen) diamond(ctx, sx, sy, 4.5);
      else ctx.arc(sx, sy, 3.5, 0, Math.PI * 2);
      ctx.stroke();
      if (m.w.citizen) {
        ctx.lineWidth = 1.5;
        ctx.strokeStyle = p.citizen;
        ctx.stroke();
      } else {
        ctx.fillStyle = p.person;
        ctx.fill();
      }
    }
    ctx.font = '600 12px "Source Sans 3", system-ui, sans-serif';
    for (const [m, sx, sy] of marked) {
      ctx.lineWidth = 1.5;
      ctx.strokeStyle = p.ink;
      ctx.beginPath();
      ctx.arc(sx, sy, 9, 0, Math.PI * 2);
      ctx.stroke();
      label(ctx, p, m.w.name, sx + 12, sy + 4);
    }
  }

  function diamond(ctx, x, y, r) {
    ctx.moveTo(x, y - r);
    ctx.lineTo(x + r, y);
    ctx.lineTo(x, y + r);
    ctx.lineTo(x - r, y);
    ctx.closePath();
  }

  function label(ctx, p, s, x, y) {
    ctx.lineWidth = 3;
    ctx.strokeStyle = p.halo;
    ctx.strokeText(s, x, y);
    ctx.fillStyle = p.ink;
    ctx.fillText(s, x, y);
  }

  const ZONE_NAMES = {
    capitol: "Capitol", trade_district: "Trade", guild_district: "Guilds", urban: "Urban",
    suburban: "Suburbs", rural: "Farms", wilderness: "The wilds",
  };
  function zoneLabels(ctx, p, zones) {
    ctx.font = '600 10px "Source Sans 3", system-ui, sans-serif';
    ctx.textAlign = "center";
    ctx.fillStyle = p.label;
    for (const z of zones) {
      const name = ZONE_NAMES[z.zone];
      if (!name || (z.outer - z.inner) * map.view.scale < 14) continue;
      // Between two arterials, north by north-east, so no road runs through it
      const r = (z.inner + z.outer) / 2;
      const a = (-67.5 * Math.PI) / 180;
      const [sx, sy] = toScreen(map.terrain.cx + r * Math.cos(a), map.terrain.cy + r * Math.sin(a));
      ctx.fillText(name.toUpperCase().split("").join(" "), sx, sy + 3);
    }
    ctx.textAlign = "start";
  }

  /** Where the trade happened: the hall's fills at its door, each stall's sales at the stall */
  function heatSpots() {
    const t = map.terrain;
    const h = map.layers.heat;
    const spots = [];
    const hall = t.landmarks.get("exchange_hall");
    if (hall && h.market.trades) spots.push({ at: hall.door, trades: h.market });
    for (const [id, trades] of Object.entries(h.stalls)) {
      const stall = t.landmarks.get(id);
      if (stall) spots.push({ at: stall.position, trades });
    }
    return spots;
  }
  function heat(ctx, p) {
    const spots = heatSpots();
    const most = Math.max(1, ...spots.map((s) => s.trades.strands));
    ctx.save();
    ctx.globalCompositeOperation = p.heatMode;
    for (const s of spots) {
      const [sx, sy] = toScreen(...s.at);
      // Area by Strands, and never smaller than a thumbprint
      const r = Math.max(10, Math.sqrt(s.trades.strands / most) * Math.max(36, 90 * map.view.scale));
      const g = ctx.createRadialGradient(sx, sy, 0, sx, sy, r);
      g.addColorStop(0, p.heat);
      g.addColorStop(1, "transparent");
      ctx.fillStyle = g;
      ctx.beginPath();
      ctx.arc(sx, sy, r, 0, Math.PI * 2);
      ctx.fill();
    }
    ctx.restore();
  }

  function scaleBar(ctx, p) {
    // The roundest length that's under 120 px
    const units = [10, 25, 50, 100, 250, 500, 1000, 2500].reverse().find((u) => u * map.view.scale <= 120) ?? 10;
    const w = units * map.view.scale;
    const x = 14;
    const y = map.size.h - 14;
    ctx.strokeStyle = p.ink;
    ctx.lineWidth = 1.5;
    ctx.beginPath();
    ctx.moveTo(x, y - 5);
    ctx.lineTo(x, y);
    ctx.lineTo(x + w, y);
    ctx.lineTo(x + w, y - 5);
    ctx.stroke();
    ctx.font = '12px "Source Sans 3", system-ui, sans-serif';
    label(ctx, p, `${nf.format(units)}`, x + w + 6, y);
  }

  // ---- what's under the cursor

  function hitAt(sx, sy) {
    let best = null;
    let bestD = 10;
    const consider = (kind, x, y, thing) => {
      const [px_, py_] = toScreen(x, y);
      const d = Math.hypot(px_ - sx, py_ - sy);
      if (d < bestD) { best = { kind, thing, id: thing.player_id ?? thing.id }; bestD = d; }
    };
    if (map.on.has("people")) for (const m of map.walkers.values()) consider("walker", ...walkerAt(m), m.w);
    if (best) return best;
    if (map.on.has("stations") && map.layers) for (const s of map.layers.stations) consider("station", s.x, s.y, s);
    if (best) return best;
    if (map.on.has("nodes") && map.nodes) for (const n of map.nodes) consider("node", n.x, n.y, n);
    return best;
  }

  function doing(w) {
    switch (w.doing.kind) {
      case "extracting": {
        const node = map.nodes?.find((n) => n.id === w.doing.node_id);
        return node ? `gathering at ${article(item(node.resource_type))}` : "gathering";
      }
      case "crafting": return `making ${item(w.doing.recipe_id)}`;
      default:
        if (!w.dest) return "standing still";
        return `walking, ${nf.format(Math.round(Math.hypot(w.dest[0] - w.x, w.dest[1] - w.y)))} to go`;
    }
  }
  const article = (s) => `${/^[aeiou]/.test(s) ? "an" : "a"} ${s}`;

  function mapNote() {
    const note = $("map-note");
    const h = map.hover;
    if (map.follow) {
      const m = map.walkers.get(map.follow);
      if (m) {
        // A citizen's name goes to their page (D4.3)
        return void note.replaceChildren(
          "Following ",
          ...(m.w.citizen ? ["◇ ", citizenLink(m.w.name)] : [m.w.name]),
          `, ${doing(m.w)}. Click the map to let go.`,
        );
      }
    }
    if (!h) return void (note.textContent = "Point at anyone to see what they're doing; click to follow them.");
    switch (h.kind) {
      case "walker": note.textContent = `${h.thing.citizen ? "◇ " : ""}${h.thing.name}${h.thing.citizen ? ", a citizen," : ""} ${doing(h.thing)}. Click to follow.`; break;
      case "station": note.textContent = `${capital(item(h.thing.station_type))}, ${h.thing.citizen ? "a citizen's" : "a person's"}.`; break;
      case "node": note.textContent = `${capital(item(h.thing.resource_type))}, ${h.thing.state}.`; break;
    }
  }

  // ---- the words over and under it

  function mapHeadline() {
    if (state.tab !== "map") return;
    const all = [...map.walkers.values()].map((m) => m.w);
    const citizens = all.filter((w) => w.citizen).length;
    const people = all.length - citizens;
    if (!all.length) return void ($("map-headline").textContent = "Nobody is about. The roads are empty.");
    const n = { walking: 0, gathering: 0, crafting: 0 };
    for (const w of all) {
      if (w.doing.kind === "extracting") n.gathering++;
      else if (w.doing.kind === "crafting") n.crafting++;
      else if (w.dest) n.walking++;
    }
    const who = citizens && people ? `${plural(citizens, "citizen", "citizens")} and ${plural(people, "person", "people")} are`
      : citizens ? `${plural(citizens, "citizen is", "citizens are")}` : `${plural(people, "person is", "people are")}`;
    const parts = [
      n.walking && `${nf.format(n.walking)} walking`,
      n.gathering && `${nf.format(n.gathering)} gathering`,
      n.crafting && `${nf.format(n.crafting)} at a craft`,
    ].filter(Boolean);
    $("map-headline").textContent = parts.length ? `${who} out: ${parts.join(", ")}.` : `${who} out, and all of them standing still.`;
  }

  function mapKey() {
    const p = palette();
    const items = [];
    const swatch = (cls, color, words) => {
      const li = document.createElement("li");
      const i = document.createElement("i");
      i.className = cls;
      i.style.setProperty("--c", color);
      li.append(i, words);
      items.push(li);
    };
    const walkers = [...map.walkers.values()];
    if (map.on.has("people")) {
      swatch("diamond", p.citizen, `Citizens ${nf.format(walkers.filter((m) => m.w.citizen).length)}`);
      swatch("dot", p.person, `People ${nf.format(walkers.filter((m) => !m.w.citizen).length)}`);
    }
    if (map.on.has("nodes") && map.nodes) {
      const by = new Map();
      for (const n of map.nodes) by.set(n.resource_type, (by.get(n.resource_type) ?? 0) + 1);
      for (const [type, n] of [...by].sort((a, b) => b[1] - a[1])) swatch("dot", p.nodes[type] || p.otherNode, `${capital(item(type))} ${nf.format(n)}`);
    }
    if (map.on.has("plots") && map.layers) {
      const c = map.layers.plots.filter((x) => x.citizen).length;
      swatch("lot", p.ownedCitizen, `Citizens' lots ${nf.format(c)}`);
      swatch("lot", p.ownedPerson, `People's lots ${nf.format(map.layers.plots.length - c)}`);
    }
    if (map.on.has("stations") && map.layers) swatch("square", p.station, `Stations ${nf.format(map.layers.stations.length)}`);
    if (map.on.has("heat") && map.layers) {
      const h = map.layers.heat;
      const stalls = Object.values(h.stalls);
      const sales = stalls.reduce((n, s) => n + s.trades, 0);
      const strands = stalls.reduce((n, s) => n + s.strands, 0);
      swatch("glow", p.heat, h.market.trades + sales === 0
        ? `No trade in ${window_()}`
        : `${plural(h.market.trades, "fill", "fills")} at the hall for ${nf.format(h.market.strands)} Strands, and ${plural(sales, "sale", "sales")} at ${plural(stalls.length, "stall", "stalls")} for ${nf.format(strands)}, in ${window_()}`);
    }
    $("map-key").replaceChildren(...items);
  }

  // ---- reading it

  async function readPositions() {
    try { takePositions(await getJson("/admin/positions")); } catch (e) { if (!(e instanceof SignedOut)) console.warn("[observatory] positions", e); }
    mapKey();
    mapNote();
  }
  async function readLayers() {
    try { map.layers = await getJson(`/admin/map?hours=${state.hours}`); } catch (e) { if (!(e instanceof SignedOut)) console.warn("[observatory] map", e); }
    mapKey();
    drawSoon();
  }
  async function readNodes() {
    try { ({ nodes: map.nodes } = await getJson("/world/nodes")); } catch (e) { if (!(e instanceof SignedOut)) console.warn("[observatory] nodes", e); }
    mapKey();
    drawSoon();
  }
  /** After a deed or a fill: the layers again, once things settle */
  function layersSoon() {
    if (state.tab !== "map" || map.pending) return;
    map.pending = setTimeout(() => { map.pending = null; readLayers(); }, 3_000);
  }

  async function mapPage() {
    if (!map.terrain) {
      map.loading ??= fetch("/terrain")
        .then((res) => { if (!res.ok) throw new Error(`/terrain: ${res.status}`); return res.json(); })
        .then((t) => { map.terrain = buildTerrain(t); })
        .finally(() => { map.loading = null; });
      $("map-headline").textContent = "Reading the land.";
      try { await map.loading; } catch (e) {
        console.warn(e);
        $("map-headline").textContent = "The land couldn't be read from /terrain.";
        return;
      }
      if (state.tab !== "map") return;
    }
    sizeMap();
    if (!map.view) fitView();
    // Sent from a citizen's page: follow them, close enough to see the streets
    if (map.want) {
      map.follow = map.want;
      map.want = null;
      if (!map.on.has("people")) setLayer("people", true);
      map.view.scale = Math.max(map.view.scale, Math.min(map.size.w, map.size.h) / FOLLOW_SPAN);
    }
    if (!map.timers.length) {
      readPositions();
      readLayers();
      readNodes();
      map.timers = [
        setInterval(() => { if (!document.hidden) readPositions(); }, POSITIONS_MS),
        setInterval(() => { if (!document.hidden) readLayers(); }, MAP_LAYERS_MS),
        setInterval(() => { if (!document.hidden) readNodes(); }, NODES_MS),
      ];
    }
    mapHeadline();
    mapKey();
    mapNote();
    drawSoon();
  }

  function mapStop() {
    for (const t of map.timers) clearInterval(t);
    map.timers = [];
    clearTimeout(map.pending);
    map.pending = null;
  }

  function sizeMap() {
    const canvas = $("map-canvas");
    const box = canvas.parentElement.getBoundingClientRect();
    if (!box.width) return;
    const dpr = devicePixelRatio || 1;
    const was = Math.min(map.size.w, map.size.h);
    map.size = { w: box.width, h: box.height };
    canvas.width = Math.round(box.width * dpr);
    canvas.height = Math.round(box.height * dpr);
    // A resize keeps what's in view the same size relative to the canvas
    if (map.view && was) map.view.scale *= Math.min(box.width, box.height) / was;
    drawSoon();
  }

  function setLayer(layer, on) {
    if (on) map.on.add(layer);
    else map.on.delete(layer);
    prefs.set("observatory.layers", JSON.stringify([...map.on]));
    for (const b of document.querySelectorAll("#layers button[data-layer]")) b.setAttribute("aria-pressed", String(map.on.has(b.dataset.layer)));
    mapKey();
    drawSoon();
  }

  function mapControls() {
    for (const b of document.querySelectorAll("#layers button[data-layer]")) {
      b.setAttribute("aria-pressed", String(map.on.has(b.dataset.layer)));
      b.addEventListener("click", () => setLayer(b.dataset.layer, !map.on.has(b.dataset.layer)));
    }
    $("map-fit").addEventListener("click", () => { if (map.terrain) { fitView(); mapNote(); drawSoon(); } });
    const canvas = $("map-canvas");
    new ResizeObserver(() => { if (state.tab === "map") sizeMap(); }).observe(canvas.parentElement);
    stillness.addEventListener("change", drawSoon);

    // Drag to move, wheel or pinch to zoom, click to follow
    const pointers = new Map();
    let moved = 0;
    let pinch = null;
    const local = (e) => { const r = canvas.getBoundingClientRect(); return [e.clientX - r.left, e.clientY - r.top]; };
    canvas.addEventListener("pointerdown", (e) => {
      canvas.setPointerCapture(e.pointerId);
      pointers.set(e.pointerId, local(e));
      moved = 0;
      if (pointers.size === 2) {
        const [a, b] = [...pointers.values()];
        pinch = { d: Math.hypot(a[0] - b[0], a[1] - b[1]) };
      }
    });
    canvas.addEventListener("pointermove", (e) => {
      const at = local(e);
      const was = pointers.get(e.pointerId);
      if (!was || !map.view) {
        if (map.view && e.pointerType === "mouse") {
          const hit = hitAt(...at);
          if (hit?.id !== map.hover?.id) { map.hover = hit; mapNote(); drawSoon(); }
          canvas.style.cursor = hit?.kind === "walker" ? "pointer" : "grab";
        }
        return;
      }
      pointers.set(e.pointerId, at);
      if (pointers.size === 2 && pinch) {
        const [a, b] = [...pointers.values()];
        const d = Math.hypot(a[0] - b[0], a[1] - b[1]);
        zoomAt((a[0] + b[0]) / 2, (a[1] + b[1]) / 2, d / pinch.d);
        pinch.d = d;
        moved += 10;
        return;
      }
      const dx = at[0] - was[0];
      const dy = at[1] - was[1];
      moved += Math.abs(dx) + Math.abs(dy);
      if (moved > 4) {
        map.follow = null;
        map.view.cx -= dx / map.view.scale;
        map.view.cy -= dy / map.view.scale;
        canvas.style.cursor = "grabbing";
        drawSoon();
      }
    });
    const up = (e) => {
      if (!pointers.has(e.pointerId)) return;
      pointers.delete(e.pointerId);
      if (pointers.size < 2) pinch = null;
      canvas.style.cursor = "grab";
      if (e.type === "pointerup" && moved <= 4 && map.view && pointers.size === 0) {
        const hit = hitAt(...local(e));
        map.follow = hit?.kind === "walker" ? hit.id : null;
        map.hover = hit;
        mapNote();
        drawSoon();
      }
    };
    canvas.addEventListener("pointerup", up);
    canvas.addEventListener("pointercancel", up);
    canvas.addEventListener("pointerleave", () => { if (map.hover && !pointers.size) { map.hover = null; mapNote(); drawSoon(); } });
    canvas.addEventListener("wheel", (e) => {
      if (!map.view) return;
      e.preventDefault();
      zoomAt(...local(e), Math.exp(-e.deltaY * (e.deltaMode === 1 ? 0.05 : 0.0015)));
    }, { passive: false });
    canvas.addEventListener("keydown", (e) => {
      if (!map.view) return;
      const step = 80 / map.view.scale;
      const keys = { ArrowLeft: [-step, 0], ArrowRight: [step, 0], ArrowUp: [0, -step], ArrowDown: [0, step] };
      if (keys[e.key]) { map.follow = null; map.view.cx += keys[e.key][0]; map.view.cy += keys[e.key][1]; drawSoon(); }
      else if (e.key === "+" || e.key === "=") zoomAt(map.size.w / 2, map.size.h / 2, 1.25);
      else if (e.key === "-") zoomAt(map.size.w / 2, map.size.h / 2, 0.8);
      else if (e.key === "0") { fitView(); mapNote(); drawSoon(); }
      else return;
      e.preventDefault();
    });
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
    $("wire").replaceChildren(...(shown.length ? shown.map(wireItem) : [emptyWire()]));
  }

  // The overview's few newest, fills folded together when several come at
  // once so a busy market doesn't push the notes off
  const LATEST = 8;
  function latestWire() {
    const from = rangeFrom();
    const recent = state.wire.filter((e) => e.at_ms > from);
    const notes = recent.filter((e) => e.kind === "annotation").length;
    const warnings = recent.filter((e) => e.group === "warning").length;
    $("latest-headline").textContent = recent.length === 0
      ? `Nothing on the wire in ${window_()}.`
      : `${plural(notes, "note", "notes")} and ${plural(warnings, "warning", "warnings")} in ${window_()}, among ${plural(recent.length, "thing", "things")} on the wire.`;
    const shown = [];
    for (let i = state.wire.length - 1; i >= 0 && shown.length < LATEST; i--) {
      const e = state.wire[i];
      const last = shown.at(-1);
      if (e.group === "fill" && last?.group === "fill") {
        last.folded = (last.folded ?? 1) + 1;
        last.text = `${plural(last.folded, "fill", "fills")} on the market, the newest: ${last.first}`;
        continue;
      }
      shown.push({ ...e, first: e.text });
    }
    $("latest-wire").replaceChildren(...(shown.length ? shown.map(wireItem) : [emptyWire()]));
  }

  function wireItem(e) {
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
  }
  const emptyWire = () => Object.assign(document.createElement("li"), { className: "empty", textContent: "Nothing yet. The wire fills as things happen." });

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

  const PAGES = { overview, map: mapPage, economy, performance, town: () => town(), citizen, tuning, events };
  // A page that isn't a tab of its own lights its parent's
  const TAB_OF = { citizen: "town" };

  /** Redraw the open page if it's one of `pages` */
  function render(pages) {
    if (pages.includes(state.tab)) {
      Promise.resolve(PAGES[state.tab]()).catch((e) => { if (!(e instanceof SignedOut)) console.warn(e); });
    }
  }

  /** A page from the hash: `town`, or `citizen/<name>` for one citizen's (D4.3) */
  function show(hash) {
    const cut = hash.indexOf("/");
    let tab = cut < 0 ? hash : hash.slice(0, cut);
    const arg = cut < 0 ? "" : decodeURIComponent(hash.slice(cut + 1)).trim();
    if (tab === "citizen") {
      if (arg) openCitizen(arg);
      else tab = "town";
    }
    if (tab !== "citizen") document.title = "The Observatory";
    if (!PAGES[tab]) tab = "overview";
    state.tab = tab;
    if (tab !== "map") mapStop();
    for (const a of document.querySelectorAll("#tabs a")) a.setAttribute("aria-selected", String(a.dataset.tab === (TAB_OF[tab] ?? tab)));
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
    // The map's heat covers the range too
    if (state.tab === "map" && map.terrain) readLayers();
    else render([state.tab]);
  }

  function setTheme(theme) {
    if (theme) document.documentElement.dataset.theme = theme;
    else delete document.documentElement.dataset.theme;
    for (const chart of Object.values(state.charts)) chart.rebuild();
    map.palette = null;
    mapKey();
    drawSoon();
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
    mapControls();
    rosterControls();
    rolesControls();
    stylesControls();
    tuningControls();
    // The link goes to the map; this says whom to follow there
    $("citizen-follow").addEventListener("click", () => { map.want = citizenView.detail?.player_id ?? null; });
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
