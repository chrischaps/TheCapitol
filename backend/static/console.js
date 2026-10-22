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
// against target.

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
    covered: { base: Infinity, load: Infinity, money: Infinity },
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
    state.covered = { base: Infinity, load: Infinity, money: Infinity };
    state.annotations.clear();
    state.wire = [];
    mapStop();
    map.walkers.clear();
    map.layers = map.nodes = null;
    map.follow = map.hover = null;
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
  // sample, so it's read only for the performance tab (#411), and the
  // money's flows and trades, and the wealth by trade, only for the economy tab
  const FIELDS = { base: "players,strands,drift", load: "load", money: "strand_flows,new_accounts,market,bazaar,distribution,books" };

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
          state.covered = { base: Infinity, load: Infinity, money: Infinity };
          await Promise.all([
            readSeries("base"),
            state.tab === "performance" ? readSeries("load") : null,
            state.tab === "economy" ? readSeries("money") : null,
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
        render(["overview", "economy", "performance", "town"]);
        break;
      }
      case "annotation": {
        const { type, ...a } = feed;
        if (addAnnotation(a)) {
          for (const chart of Object.values(state.charts)) chart.redraw();
          if (a.kind === "tunable_changed") loadKnobs();
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
          x: { time: true, range: () => [rangeFrom() / 1000, Date.now() / 1000] },
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

    // On a log scale, a label and a rule at each power of ten and none between
    function decades(axis) {
      const filter = (u, splits) => splits.map((v) => (Math.abs(Math.log10(v) % 1) < 1e-9 ? v : null));
      return { filter, grid: { ...axis.grid, filter }, ticks: { ...axis.ticks, filter } };
    }

    function data() {
      // Rows a spec works out itself, such as bins: `x` in Unix seconds,
      // `values` drawn and `own` the legend's, each by series key
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
    spread: () => lineChart($("chart-spread"), {
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
  CHARTS.flows = () => lineChart($("chart-flows"), {
    rows: () => money.rows,
    axis: short,
    legend: (v) => `${v > 0 ? "+" : ""}${count(v)}`,
    // Drawn outermost first, so each bar's inner lines paint over it
    series: () => money.lines.map((l) => ({ key: l.key, label: l.label, color: l.color, bars: true })).reverse(),
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
  const PAGE_CHARTS = { overview: ["players"], economy: ["strands", "flows", "escrow", "velocity", "spread", "prices"], performance: ["tick"] };

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
  CHARTS.prices = () => lineChart($("chart-prices"), {
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
      state.charts.prices?.destroy();
      delete state.charts.prices;
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
  // The tick's tiles read `load`, which the series sends only for the
  // performance tab (#411), so until that tab is opened their sparklines
  // hold what the stream has brought since the page opened.
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
    if (state.covered.money > rangeFrom() || !books) {
      if (!books) $("flows-headline").textContent = "Reading the books.";
      try {
        books ??= await getJson("/admin/books");
        await readSeries("money");
      } catch (e) { if (e instanceof SignedOut) return; console.warn(e); }
    }
    strands();
    if (books) flows();
    escrow();
    distribution();
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
      $("roles").tBodies[0].replaceChildren(emptyRow(6, "No sample says yet."));
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

  const trade = (role) => role === "person" ? "People" : role === "citizen" ? "Citizens with no trade named" : capital(role.replace(/^([a-z]+)-(.+)$/, "$1 ($2)"));
  const each = (h) => h.strands / Math.max(h.accounts, 1);
  /** One of a trade, in a sentence: "a merchant", "a person" */
  const one = (role) => role === "person" ? "a person" : role === "citizen" ? "a citizen with no trade named" : `a ${trade(role).toLowerCase()}`;

  // The wealth table: each trade's accounts and Strands now, against the
  // first sample in the range that has the trade (the town names them only
  // once it reports in), as the soak report's table of trades has them
  function roles(d, inRange) {
    const total = Object.values(d.roles).reduce((n, h) => n + Math.max(h.strands, 0), 0);
    const rows = Object.entries(d.roles).sort((a, b) => b[1].strands - a[1].strands);
    const byEach = [...rows].sort((a, b) => each(b[1]) - each(a[1]));
    const [richest, poorest] = [byEach[0], byEach.at(-1)];
    $("roles-headline").textContent = rows.length < 2 || each(richest[1]) === each(poorest[1])
      ? `Every trade holds ${nf.format(Math.round(each(richest[1])))} Strands each.`
      : `${capital(one(richest[0]))} holds the most, ${nf.format(Math.round(each(richest[1])))} Strands on average; ${one(poorest[0])} the least, ${nf.format(Math.round(each(poorest[1])))}.`;
    $("roles").tBodies[0].replaceChildren(...rows.map(([role, h]) => {
      const before = inRange.find((d) => d.roles?.[role])?.roles[role];
      const change = before ? each(h) - each(before) : null;
      const tr = document.createElement("tr");
      tr.append(
        cell(trade(role)),
        cell(nf.format(h.accounts), "num"),
        cell(nf.format(h.strands), "num"),
        cell(nf.format(Math.round(each(h))), "num"),
        cell(change == null ? "–" : `${change > 0 ? "+" : change < 0 ? "−" : "±"}${nf.format(Math.abs(Math.round(change)))}`, `num ${change > 0 ? "up" : change < 0 ? "down" : "dim"}`),
        cell(total ? percent(Math.max(h.strands, 0) / total) : "–", "num dim"),
      );
      return tr;
    }));
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
    deeds: { label: "Deeds", say: "deeds" },
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
    if (rebuild) state.charts.flows?.rebuild();
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
    state.giniWatch = knobs.find((k) => k.key === "watch_gini")?.value ?? state.giniWatch;
    state.charts.spread?.update();
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
      if (m) return void (note.textContent = `Following ${m.w.citizen ? "◇ " : ""}${m.w.name}, ${doing(m.w)}. Click the map to let go.`);
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

  const PAGES = { overview, map: mapPage, economy, performance, town: () => town(), tuning: loadKnobs, events };

  /** Redraw the open page if it's one of `pages` */
  function render(pages) {
    if (pages.includes(state.tab)) {
      Promise.resolve(PAGES[state.tab]()).catch((e) => { if (!(e instanceof SignedOut)) console.warn(e); });
    }
  }

  function show(tab) {
    if (!PAGES[tab]) tab = "overview";
    state.tab = tab;
    if (tab !== "map") mapStop();
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
