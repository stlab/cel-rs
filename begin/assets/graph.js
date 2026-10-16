(function () {
  var root = typeof window !== "undefined" ? window : globalThis;
  // Tunable layout constants (shared across every instance).
  var LINK_DISTANCE = 80;
  var CHARGE_STRENGTH = -300;
  // Independent per-node pull toward the viewport center. A disconnected node or
  // component would otherwise drift outward under charge repulsion alone until its
  // residual velocity finally decays, often hundreds of pixels from the rest of the
  // graph. This weak forceX/forceY gives every node its own restoring pull toward
  // center so it settles at a finite distance instead.
  //
  // This is deliberately NOT d3.forceCenter: forceCenter recenters the *average*
  // position of all nodes by directly overwriting every node's x/y each tick, at
  // full strength, uncapped by alpha decay -- and it does this unconditionally,
  // ignoring fx/fy entirely. A pinned node's overwritten position gets discarded a
  // moment later by the simulation's own fx/fy enforcement, but any node that is
  // NOT pinned keeps whatever shift forceCenter just gave it, plus its usual
  // velocity-based forces on top. If most of the graph is pinned away from the
  // canvas center (e.g. every node but one dragged into a corner), forceCenter
  // recomputes essentially the same large correction every tick and dumps all of
  // it onto that one free node, compounding for as long as the simulation keeps
  // ticking -- flinging it hundreds or thousands of pixels away. forceX/forceY
  // avoid this entirely: they only ever adjust velocity (vx/vy), which the
  // simulation's own fx/fy enforcement already handles correctly for pinned nodes,
  // and their contribution is naturally scaled down by alpha like every other
  // force.
  var CENTER_PULL_STRENGTH = 0.1;
  var CELL_W = 60;
  var CELL_H = 36;
  var CELL_RX = 4;
  var CELL_LABEL_PADDING = 16;
  var REL_R = 16;
  var COND_SIZE = 20;
  var CELL_COLLIDE_R = 38;
  var REL_COLLIDE_R = 22;
  var COND_COLLIDE_R = COND_SIZE * Math.SQRT2;
  var NODE_STROKE_WIDTH = 1.5;
  var CONTROL_DOT_RADIUS = 2.4;
  var FIT_MARGIN = 16;
  var PULSE_COLOR = "#f90";
  var PULSE_ON_MS = 200;
  var PULSE_OFF_MS = 400;
  var INACTIVE_STROKE = "#ccc";

  // One GraphInstance per mounted container id -- begin's single view, or one of a
  // book page's several simultaneously-live examples. Each owns its D3
  // simulation/SVG/layout state entirely independently; nothing here is shared
  // across instances, so switching or dragging one can never affect another.
  var instances = new Map();

  // ---- Pure helpers (no instance state) ----

  /**
   * Validates an unsigned protocol counter without coercion or truncation.
   * @param {number} value Counter to validate.
   * @returns {number} The unchanged u32 value.
   * @throws {Error} When the value is not an integer in the u32 range.
   */
  function u32(value) {
    if (!Number.isInteger(value) || value < 0 || value > 4294967295)
      throw new Error("graph counter is outside u32 range");
    return value;
  }

  /**
   * Increments a protocol counter, explicitly rejecting exhaustion.
   * @param {number} value Previous u32 value.
   * @returns {number} The checked successor.
   * @throws {Error} On exhaustion or an invalid input.
   */
  function nextCounter(value) {
    return u32(u32(value) + 1);
  }

  /**
   * Owns one in-flight request and one latest complete pending snapshot.
   * Rendering occurs only for matching successful current-geometry replies;
   * pending gestures never invalidate approved progress at the same geometry.
   * @param {number} generation Explicitly started u32 lifecycle.
   * @param {function(object):void} send Ordered transport submission.
   * @param {function(object, object):void} apply Approved-output renderer and request.
   * @param {function(string):void} fail Visible/error-channel reporter.
   */
  function ProposalQueue(generation, send, apply, fail) {
    this.generation = u32(generation);
    this.send = send;
    this.apply = apply;
    this.fail = fail;
    this.sequence = 0;
    this.revision = 0;
    this.applied = 0;
    this.inflight = null;
    this.pending = null;
    this.disposed = false;
    send({ kind: "start", generation: generation });
  }

  /** Returns the bounded number of retained request snapshots (zero to two). */
  Object.defineProperty(ProposalQueue.prototype, "size", {
    get: function () { return Number(!!this.inflight) + Number(!!this.pending); },
  });

  /**
   * Retains the latest geometry and action, preserving queued Fit on refresh.
   * A newer explicit proposal intentionally replaces a queued Fit.
   * @param {object} snapshot Complete geometry and action, without sequence.
   * @postcondition At most two snapshots are retained, independent of burst size.
   */
  ProposalQueue.prototype.submit = function (snapshot) {
    if (this.disposed) return;
    try {
      u32(snapshot.revision);
      if (snapshot.revision < this.revision) return;
      this.revision = snapshot.revision;
      var action = snapshot.action;
      if (action.kind === "refresh") {
        if (this.pending) action = this.pending.action;
        else if (this.inflight && this.inflight.action.kind === "fit")
          action = this.inflight.action;
      }
      this.pending = {
        generation: this.generation, revision: snapshot.revision,
        bounds: snapshot.bounds.slice(), viewport: snapshot.viewport.slice(),
        action: action.kind === "propose"
          ? { kind: "propose", view: { x: action.view.x, y: action.view.y, k: action.view.k } }
          : { kind: action.kind },
      };
      this.pump();
    } catch (error) { this.stop(error.message); }
  };

  /** Submits pending work only when the single in-flight slot is vacant. */
  ProposalQueue.prototype.pump = function () {
    if (this.disposed || this.inflight || !this.pending) return;
    this.sequence = nextCounter(this.sequence);
    this.inflight = this.pending;
    this.pending = null;
    this.inflight.sequence = this.sequence;
    this.send({ kind: "update", request: this.inflight });
  };

  /**
   * Releases matching work even when its geometry is too old to render.
   * @param {object} reply Typed model reply.
   * @postcondition Unrelated generation/sequence replies cannot clear a live slot.
   */
  ProposalQueue.prototype.receive = function (reply) {
    if (this.disposed) return;
    try {
      if (!reply) throw new Error("invalid graph reply");
      u32(reply.generation);
      if (reply.generation !== this.generation) return;
      u32(reply.sequence); u32(reply.revision);
      if (!this.inflight || reply.sequence !== this.inflight.sequence) return;
      if (reply.revision !== this.inflight.revision)
        throw new Error("graph reply revision does not match its request");
      var request = this.inflight;
      this.inflight = null;
      if (reply.kind !== "accepted" && reply.kind !== "failed")
        throw new Error("unknown graph reply");
      if (reply.revision < this.revision) { this.pump(); return; }
      if (reply.kind === "failed") throw new Error(reply.message);
      if (!validOutput(reply.output)) throw new Error("invalid approved graph output");
      if (reply.revision === this.revision && reply.sequence > this.applied) {
        this.applied = reply.sequence;
        this.apply(reply.output, request);
      }
      this.pump();
    } catch (error) { this.stop(error.message); }
  };

  /** Retires the queue and exposes an error without any fallback rendering. */
  ProposalQueue.prototype.stop = function (message) {
    this.retire();
    this.fail(message);
  };

  /** Ends this generation once and discards all retained work. */
  ProposalQueue.prototype.dispose = function () {
    var error = this.retire();
    if (error) this.fail(error);
  };

  /** Retires work and returns any transport diagnostic without throwing. */
  ProposalQueue.prototype.retire = function () {
    if (this.disposed) return;
    this.disposed = true;
    this.pending = this.inflight = null;
    try { this.send({ kind: "dispose", generation: this.generation }); }
    catch (error) { return error.message; }
  };

  /**
   * Validates finite approved transforms and consistent model-owned zoom hints.
   * @param {object} output Candidate model output.
   * @returns {boolean} Whether every required number satisfies the wire contract.
   */
  function validOutput(output) {
    /** Returns whether a transform has finite pan and positive finite zoom. */
    function validView(view) {
      return view && Number.isFinite(view.x) && Number.isFinite(view.y)
        && Number.isFinite(view.k) && view.k > 0;
    }
    return !!(output && validView(output.view) && validView(output.fit)
      && Number.isFinite(output.min_zoom) && output.min_zoom > 0
      && Number.isFinite(output.max_zoom) && output.max_zoom >= output.min_zoom
      && output.view.k >= output.min_zoom && output.view.k <= output.max_zoom);
  }

  /**
   * Creates a container-scoped ordered transport with monotonic generations.
   * @param {function(object):void} send Rust evaluation-channel sender.
   * @param {function(string):void} fail Visible diagnostic reporter.
   * @returns {object} Lifecycle allocator, subscriptions, delivery and disposal.
   */
  function createChannel(send, fail) {
    var generation = 0, subscribers = new Set(), closed = false;
    return {
      send: send,
      fail: fail,
      /** Allocates the next checked lifecycle without reusing a retired number. */
      nextGeneration: function () {
        if (closed) throw new Error("graph channel is closed");
        generation = nextCounter(generation);
        return generation;
      },
      /** Registers a reply receiver and returns its idempotent disposer. */
      subscribe: function (receive) {
        if (closed) throw new Error("graph channel is closed");
        subscribers.add(receive);
        return function () { subscribers.delete(receive); };
      },
      /**
       * Delivers only to currently subscribed instances of this channel.
       * @complexity O(s) callbacks for s live subscriptions.
       */
      deliver: function (reply) {
        if (!closed) subscribers.forEach(function (receive) { receive(reply); });
      },
      /**
       * Closes delivery and releases every subscription.
       * @complexity O(s) released subscriptions.
       */
      close: function () { closed = true; subscribers.clear(); },
      /** Returns the number of live instance subscriptions. */
      get subscriberCount() { return subscribers.size; },
    };
  }

  function setsEqual(a, b) {
    if (a.size !== b.size) return false;
    for (var v of a) {
      if (!b.has(v)) return false;
    }
    return true;
  }

  /**
   * Returns the point where a source-to-target ray exits a rectangle.
   * The rectangle is centered at the target and uses the supplied positive
   * half-width and half-height; omitted dimensions default to `CELL_W / 2`
   * (`30`) and `CELL_H / 2` (`18`).
   * @param {number} sx Source x-coordinate.
   * @param {number} sy Source y-coordinate.
   * @param {number} tx Target x-coordinate.
   * @param {number} ty Target y-coordinate.
   * @param {number} [hw] Rectangle half-width.
   * @param {number} [hh] Rectangle half-height.
   * @returns {{x: number, y: number}} A point on the rectangle boundary in the
   * target-facing direction, or exactly `{x: tx, y: ty}` when the distance is
   * less than one unit.
   * @postcondition For non-near-zero input, the returned point lies on the
   * rectangle boundary selected by the smaller normalized-axis distance.
   * @complexity O(1) time and space.
   */
  function cellEdgePoint(sx, sy, tx, ty, hw, hh) {
    if (hw === undefined) hw = CELL_W / 2;
    if (hh === undefined) hh = CELL_H / 2;
    var dx = tx - sx,
      dy = ty - sy;
    var dist = Math.sqrt(dx * dx + dy * dy);
    if (dist < 1) return { x: tx, y: ty };
    var nx = dx / dist,
      ny = dy / dist;
    var td = Math.abs(nx) > 1e-9 ? hw / Math.abs(nx) : Infinity;
    var ld = Math.abs(ny) > 1e-9 ? hh / Math.abs(ny) : Infinity;
    var d = Math.min(td, ld);
    return { x: tx - nx * d, y: ty - ny * d };
  }

  function cellWidth(d) {
    return d.w || CELL_W;
  }

  /**
   * Returns the point where a source-to-center ray exits a circle.
   * The circle is centered at `(cx, cy)` and has the supplied positive radius.
   * @param {number} sx Source x-coordinate.
   * @param {number} sy Source y-coordinate.
   * @param {number} cx Circle center x-coordinate.
   * @param {number} cy Circle center y-coordinate.
   * @param {number} r Circle radius.
   * @returns {{x: number, y: number}} A point exactly `r` units from the center
   * along the source-facing radial direction, or exactly `{x: cx, y: cy}` when
   * the distance is less than one unit.
   * @postcondition For non-near-zero input, the returned point lies on the
   * circle boundary.
   * @complexity O(1) time and space.
   */
  function circleEdgePoint(sx, sy, cx, cy, r) {
    var dx = cx - sx,
      dy = cy - sy;
    var dist = Math.sqrt(dx * dx + dy * dy);
    if (dist < 1) return { x: cx, y: cy };
    return { x: cx - (dx / dist) * r, y: cy - (dy / dist) * r };
  }

  /**
   * Computes clipped endpoints for a graph link.
   * `Cell` nodes use their rectangle (`w` or `CELL_W`, by `CELL_H`); `Relationship`
   * nodes use `REL_R`; `Conditional` nodes use `COND_COLLIDE_R`; and `Branch`
   * nodes are not clipped.
   * @param {{source: {kind: string, x: number, y: number, w?: number}, target: {kind: string, x: number, y: number, w?: number}}} d Link with resolved node objects.
   * @returns {{x1: number, y1: number, x2: number, y2: number}} Clipped endpoints.
   * @postcondition Each endpoint is clipped according to the source or target
   * node kind, and a `Branch` endpoint equals that node's coordinates.
   * @complexity O(1) time and space.
   */
  function linkEndpoints(d) {
    var s = d.source,
      t = d.target;
    function edgePt(node, ox, oy) {
      if (node.kind === "Cell")
        return cellEdgePoint(
          ox,
          oy,
          node.x,
          node.y,
          cellWidth(node) / 2,
          CELL_H / 2,
        );
      if (node.kind === "Branch") return { x: node.x, y: node.y };
      var r = node.kind === "Conditional" ? COND_COLLIDE_R : REL_R;
      return circleEdgePoint(ox, oy, node.x, node.y, r);
    }
    var srcPt = edgePt(s, t.x, t.y);
    var tgtPt = edgePt(t, s.x, s.y);
    return { x1: srcPt.x, y1: srcPt.y, x2: tgtPt.x, y2: tgtPt.y };
  }

  /**
   * Computes the visible graph bounds with the fixed `FIT_MARGIN` (`16`).
   * `Cell` geometry uses half-width `w / 2` or `CELL_W / 2` (`30`) and
   * half-height `CELL_H / 2` (`18`); `Relationship` uses `REL_R` (`16`);
   * `Conditional` uses `COND_COLLIDE_R` (`20 * sqrt(2)`); and `Branch` has
   * zero extent.
   * @param {Array<{id: string, kind: string, x: number, y: number, w?: number}>} nodes Graph nodes.
   * @param {number} width Viewport width.
   * @param {number} height Viewport height.
   * @param {Set<string>} [hiddenNodeIds] Node ids excluded from the bounds.
   * @returns {{minX: number, minY: number, maxX: number, maxY: number}} Bounds
   * expanded by `FIT_MARGIN`, or the unexpanded viewport when no node is visible.
   * @postcondition Hidden nodes do not affect the bounds.
   * @complexity O(n) time and O(1) auxiliary space.
   */
  function computeBBox(nodes, width, height, hiddenNodeIds) {
    var minX = Infinity,
      minY = Infinity,
      maxX = -Infinity,
      maxY = -Infinity;
    nodes.forEach(function (n) {
      if (hiddenNodeIds && hiddenNodeIds.has(n.id)) return;
      var hw, hh;
      if (n.kind === "Cell") {
        hw = cellWidth(n) / 2;
        hh = CELL_H / 2;
      } else if (n.kind === "Conditional") {
        hw = COND_COLLIDE_R;
        hh = COND_COLLIDE_R;
      } else if (n.kind === "Branch") {
        hw = 0;
        hh = 0;
      } else {
        hw = REL_R;
        hh = REL_R;
      }
      minX = Math.min(minX, n.x - hw);
      minY = Math.min(minY, n.y - hh);
      maxX = Math.max(maxX, n.x + hw);
      maxY = Math.max(maxY, n.y + hh);
    });
    if (!isFinite(minX)) {
      return { minX: 0, minY: 0, maxX: width, maxY: height };
    }
    return {
      minX: minX - FIT_MARGIN,
      minY: minY - FIT_MARGIN,
      maxX: maxX + FIT_MARGIN,
      maxY: maxY + FIT_MARGIN,
    };
  }

  /**
   * Reconciles graph data while retaining existing node objects and positions.
   * @param {{nodes: Array<object>, links: Array<object>}} previous Previous graph.
   * @param {{nodes: Array<object>, links: Array<object>}} next Incoming graph.
   * @returns {{nodes: Array<object>, links: Array<object>, relabeledIds: Set<string>, structureChanged: boolean}} Reconciled graph.
   * @complexity O(n + m) time and O(n + m) space.
   */
  function reconcileNodes(previous, next) {
    /**
     * Produces an order-independent key for an undirected link.
     * @param {string|{id: string}} a First endpoint identifier or node.
     * @param {string|{id: string}} b Second endpoint identifier or node.
     * @returns {string} A stable key containing both endpoint identifiers.
     * @postcondition Reversing the endpoints produces the same key.
     * @complexity O(1) time and space.
     */
    function linkKey(a, b) {
      var source = typeof a === "object" ? a.id : a;
      var target = typeof b === "object" ? b.id : b;
      return source < target ? source + "|" + target : target + "|" + source;
    }
    var oldNodeIds = new Set(
      previous.nodes.map(function (n) {
        return n.id;
      }),
    );
    var oldLinkSet = new Set(
      previous.links.map(function (l) {
        return linkKey(l.source, l.target);
      }),
    );
    var structureChanged =
      previous.nodes.length !== next.nodes.length ||
      previous.links.length !== next.links.length ||
      next.nodes.some(function (n) {
        return !oldNodeIds.has(n.id);
      }) ||
      next.links.some(function (l) {
        return !oldLinkSet.has(linkKey(l.source, l.target));
      });
    var oldNodeMap = new Map(
      previous.nodes.map(function (n) {
        return [n.id, n];
      }),
    );
    var relabeledIds = new Set();
    var nodes = next.nodes.map(function (n) {
      var existing = oldNodeMap.get(n.id);
      if (!existing) return Object.assign({}, n);
      if (existing.label !== n.label) relabeledIds.add(n.id);
      existing.kind = n.kind;
      existing.label = n.label;
      existing.value = n.value;
      return existing;
    });
    return {
      nodes: nodes,
      links: next.links.map(function (l) {
        return Object.assign({}, l);
      }),
      relabeledIds: relabeledIds,
      structureChanged: structureChanged,
    };
  }

  function dragBehavior(sim) {
    return d3
      .drag()
      .on("start", function (event) {
        // Both d3.zoom (on the <svg>) and this drag (on the node
        // shape) listen for pointer-down; without this the same
        // gesture would also pan the canvas.
        event.sourceEvent.stopPropagation();
      })
      .on("drag", function (event, d) {
        if (!d3.select(this).classed("dragging")) {
          sim.alphaTarget(0.3).restart();
          d3.select(this).classed("dragging", true);
        }
        d.fx = event.x;
        d.fy = event.y;
      })
      .on("end", function (event) {
        if (!event.active) sim.alphaTarget(0);
        d3.select(this).classed("dragging", false);
      });
  }

  // ---- GraphInstance: one independent D3 force layout mounted into one container ----

  /**
   * Owns one graph's D3 resources and lifecycle-scoped approval queue.
   * @param {string} containerId Mounted container id.
   * @param {object} channel Container-owned bidirectional evaluation channel.
   */
  function GraphInstance(containerId, channel) {
    this.containerId = containerId;
    this.svg = null;
    this.simulation = null;
    this.controlLinkLayer = null;
    this.linkLayer = null;
    this.cellLayer = null;
    this.relLayer = null;
    this.condLayer = null;
    this.labelLayer = null;
    this.valueLayer = null;
    this.nodes = [];
    this.links = [];
    this.width = 800;
    this.height = 600;
    this.resizeObserver = null;
    this.zoom = null;
    this.zoomLayer = null;
    this.hasInitialFit = false;
    this.initialFitRequested = false;
    this.latestData = null;
    // Seeded from a JS global that begin (not graph.js/GraphView -- see their
    // doc comments) sets alongside its "Show inactive" toggle, mirroring the
    // existing window.__beginGraphData seam: this lets a fresh instance (built
    // by init() on a source switch) start from whatever value the toggle is
    // currently showing instead of always resetting to true, without threading
    // a begin-only concern through GraphView's props. Falls back to true (dim,
    // not hide) when the global is unset -- e.g. every book page, which has no
    // such toggle.
    this.showInactive =
      typeof root.__beginShowInactive === "boolean"
        ? root.__beginShowInactive
        : true;
    this.hiddenNodeIds = new Set();
    this.channel = channel;
    this.synchronizing = false;
    this.gestureActive = false;
    this.approved = null;
    // null, "waiting", or "cancelled" until the outstanding target is consumed.
    this.resetPending = null;
    this.resetAnimating = false;
    this.geometry = null;
    this.revision = 0;
    var self = this;
    this.queue = new ProposalQueue(channel.nextGeneration(), channel.send,
      function (output, request) { self.applyApproved(output, request); }, channel.fail);
    this.unsubscribe = channel.subscribe(function (reply) { self.queue.receive(reply); });
  }

  /**
   * Returns this instance's visible-node bounds for its current viewport.
   * @returns {{minX: number, minY: number, maxX: number, maxY: number}} Bounds
   * computed with `FIT_MARGIN` and `hiddenNodeIds`.
   * @postcondition The result matches `computeBBox(this.nodes, this.width,
   * this.height, this.hiddenNodeIds)`.
   * @complexity O(n) time and O(1) auxiliary space.
   */
  GraphInstance.prototype.computeBBox = function () {
    return computeBBox(this.nodes, this.width, this.height, this.hiddenNodeIds);
  };

  /**
   * Measures geometry and submits it without deriving any view policy.
   * @param {object} action Explicit source action.
   * @complexity O(n) time and O(1) auxiliary space in visible graph nodes.
   */
  GraphInstance.prototype.submitAction = function (action) {
    if (this.queue.disposed) return;
    try {
      var box = this.computeBBox();
      var geometry = [box.minX, box.minY, box.maxX, box.maxY, this.width, this.height];
      if (!this.geometry || geometry.some(function (v, i) { return v !== this.geometry[i]; }, this)) {
        this.revision = nextCounter(this.revision);
        this.geometry = geometry;
      }
      this.queue.submit({ revision: this.revision, bounds: geometry.slice(0, 4),
        viewport: geometry.slice(4), action: action });
    } catch (error) { this.queue.stop(error.message); }
  };

  /** Submits absolute gesture proposals, except during approved baseline synchronization. */
  GraphInstance.prototype.proposeView = function (event) {
    if (!this.synchronizing) {
      if (event.sourceEvent && this.resetPending) this.resetPending = "cancelled";
      this.submitAction({ kind: "propose", view: event.transform });
    }
  };

  /** Defers programmatic baseline synchronization throughout a D3 gesture/transition. */
  GraphInstance.prototype.beginGesture = function () {
    if (!this.synchronizing) this.gestureActive = true;
  };

  /** Synchronizes the last approved baseline after the gesture/transition has ended. */
  GraphInstance.prototype.endGesture = function () {
    if (this.synchronizing) return;
    this.gestureActive = false;
    this.syncApproved();
  };

  /**
   * Renders a model-approved frame and feeds only model limits back to D3.
   * A reset-target Fit starts interpolation without snapping the visible view.
   * @param {object} output Validated complete model output.
   * @param {object} request Snapshot that produced this approval.
   * @postcondition No proposal is emitted by applying this frame.
   */
  GraphInstance.prototype.applyApproved = function (output, request) {
    this.zoom.scaleExtent([output.min_zoom, output.max_zoom]);
    // A coalesced refresh can still carry Fit while the target is in flight.
    // It is another target acknowledgement, not an interpolated display frame.
    if (this.resetAnimating && request.action.kind !== "propose") return;
    if (this.resetPending && request.action.kind === "fit") {
      var animate = this.resetPending === "waiting";
      this.resetPending = null;
      if (animate) this.animateFit(output.fit);
      return;
    }
    this.approved = output;
    this.hasInitialFit = true;
    this.zoomLayer.attr("transform", d3.zoomIdentity.translate(output.view.x, output.view.y).scale(output.view.k));
    this.zoomLayer.attr("visibility", null);
    this.syncApproved();
  };

  /** Synchronizes D3's approved baseline under a no-feedback guard, never mid-transition. */
  GraphInstance.prototype.syncApproved = function () {
    if (!this.approved || this.gestureActive || !this.svg || this.queue.disposed) return;
    this.synchronizing = true;
    try {
      var view = this.approved.view;
      this.svg.call(this.zoom.transform, d3.zoomIdentity.translate(view.x, view.y).scale(view.k));
    } finally { this.synchronizing = false; }
  };

  /**
   * Requests initial/explicit fit or a geometry-only refresh from Adam.
   * @param {boolean} [forceFit] Whether to replace the source aspiration.
   * @complexity O(n) time and O(1) auxiliary space in graph nodes.
   */
  GraphInstance.prototype.updateZoomConstraints = function (forceFit) {
    var fit = forceFit || !this.initialFitRequested;
    this.initialFitRequested = true;
    this.submitAction({ kind: fit ? "fit" : "refresh" });
  };

  GraphInstance.prototype.settleSimulation = function (forceFit) {
    var n = Math.ceil(
      Math.log(this.simulation.alphaMin()) /
        Math.log(1 - this.simulation.alphaDecay()),
    );
    this.simulation.stop().alpha(1).tick(n);
    this.ticked();
    this.updateZoomConstraints(forceFit);
  };

  // Starts this (freshly constructed) instance observing its container's size and
  // building the graph once a real size is known. Never called twice on the same
  // instance -- the public `init(id, data)` below always constructs a new
  // `GraphInstance` rather than reusing one, so there is nothing here to tear down.
  GraphInstance.prototype.start = function (data) {
    this.latestData = data;
    var self = this;
    var container = document.getElementById(this.containerId);

    // Keep observing for the life of this instance so the view area tracks the
    // container's size continuously, not just once at mount. Every firing after
    // the first just resizes the existing canvas.
    //
    // The FIRST build is deferred until the container reports a real (non-zero)
    // size: a ResizeObserver callback can fire before layout has settled, reading
    // a stale 0 width/height. Building then would fit the graph to the 800x600
    // fallback, and because a later real-size firing only calls resizeCanvas()
    // (which preserves the current view rather than re-fitting), that wrong
    // initial fit would stick -- the graph appears zoomed/cut off on first load.
    // Waiting for a real size guarantees the one-time initial fit is computed
    // against the actual viewport.
    this.resizeObserver = new ResizeObserver(function () { self.observeSize(container); });
    this.resizeObserver.observe(container);
  };

  /**
   * Builds only at a real nonzero size and ignores callbacks after disposal.
   * Later measurements preserve the live layout and submit geometry refreshes.
   * @param {HTMLElement} container This instance's observed container.
   * @complexity O(n + m + p) for initial graph construction/layout, otherwise O(n)
   * for geometry measurement, in nodes `n`, links `m`, and settling work `p`.
   */
  GraphInstance.prototype.observeSize = function (container) {
    if (this.queue.disposed) return;
    try {
      var w = container.clientWidth, h = container.clientHeight;
      if (!this.svg) {
        if (!w || !h) return;
        this.width = w;
        this.height = h;
        this.buildGraph(container, this.latestData);
      } else {
        this.width = w || this.width;
        this.height = h || this.height;
        this.resizeCanvas();
      }
    } catch (error) { this.queue.stop(error.message); }
  };

  // Resizes the existing SVG to the current width/height without touching
  // node positions or restarting the simulation.
  GraphInstance.prototype.resizeCanvas = function () {
    this.svg
      .attr("width", this.width)
      .attr("height", this.height)
      .attr("viewBox", [0, 0, this.width, this.height]);
    this.simulation.force("x").x(this.width / 2);
    this.simulation.force("y").y(this.height / 2);
    this.updateZoomConstraints();
  };

  GraphInstance.prototype.buildGraph = function (container, data) {
    var self = this;
    this.svg = d3
      .select(container)
      .append("svg")
      .attr("width", this.width)
      .attr("height", this.height)
      .attr("viewBox", [0, 0, this.width, this.height]);

    var defs = this.svg.append("defs");

    // Marker ids are namespaced per container so multiple graphs on one page don't collide:
    // SVG `url(#id)` resolves to the first matching id in the *document*, so a shared id would
    // make every graph's arrows/dots reference the first instance's markers (and break when
    // that instance is destroyed). See url() references in update().
    this.arrowMarkerId = "arrowhead-" + this.containerId;
    this.dotMarkerId = "dot-" + this.containerId;

    // Arrowhead: refX=10 places the tip (at local x=10) at the line endpoint.
    defs
      .append("marker")
      .attr("id", this.arrowMarkerId)
      .attr("viewBox", "0 -5 10 10")
      .attr("refX", 10)
      .attr("refY", 0)
      .attr("markerWidth", 8)
      .attr("markerHeight", 8)
      .attr("markerUnits", "userSpaceOnUse")
      .attr("orient", "auto")
      .append("path")
      .attr("d", "M0,-5L10,0L0,5")
      .attr("fill", "context-stroke");

    // Dot marker: caps control links where they meet the relationship they target.
    defs
      .append("marker")
      .attr("id", this.dotMarkerId)
      .attr("viewBox", "0 0 10 10")
      .attr("refX", 5)
      .attr("refY", 5)
      .attr("markerWidth", 6)
      .attr("markerHeight", 6)
      .attr("markerUnits", "userSpaceOnUse")
      .attr("orient", "auto")
      .append("circle")
      .attr("cx", 5)
      .attr("cy", 5)
      .attr("r", 4)
      .attr("fill", "context-stroke");

    // Layer z-order: bg → control links → constraint links → cells → rels → conditionals → labels → values
    this.zoomLayer = this.svg.append("g").attr("class", "zoom-layer").attr("visibility", "hidden");
    this.zoomLayer.append("g").attr("class", "bg-layer");
    this.controlLinkLayer = this.zoomLayer
      .append("g")
      .attr("class", "control-link-layer");
    this.linkLayer = this.zoomLayer.append("g").attr("class", "link-layer");
    this.cellLayer = this.zoomLayer.append("g").attr("class", "cell-layer");
    this.relLayer = this.zoomLayer.append("g").attr("class", "rel-layer");
    this.condLayer = this.zoomLayer.append("g").attr("class", "cond-layer");
    this.labelLayer = this.zoomLayer.append("g").attr("class", "label-layer");
    this.valueLayer = this.zoomLayer.append("g").attr("class", "value-layer");

    this.zoom = d3.zoom()
      // D3 owns gesture mechanics, never geometry policy.
      .constrain(function (transform) { return transform; })
      .on("start", function () { self.beginGesture(); })
      .on("zoom", function (event) { self.proposeView(event); })
      .on("end", function () { self.endGesture(); });
    this.svg.call(this.zoom);

    this.simulation = d3
      .forceSimulation()
      .force(
        "link",
        d3
          .forceLink()
          .id(function (d) {
            return d.id;
          })
          .distance(function (d) {
            var sKind = typeof d.source === "object" ? d.source.kind : null;
            var tKind = typeof d.target === "object" ? d.target.kind : null;
            return sKind === "Branch" || tKind === "Branch"
              ? LINK_DISTANCE / 2
              : LINK_DISTANCE;
          }),
      )
      .force(
        "charge",
        d3.forceManyBody().strength(function (d) {
          return d.kind === "Branch" ? 0 : CHARGE_STRENGTH;
        }),
      )
      .force("x", d3.forceX(this.width / 2).strength(CENTER_PULL_STRENGTH))
      .force("y", d3.forceY(this.height / 2).strength(CENTER_PULL_STRENGTH))
      .force(
        "collide",
        d3.forceCollide().radius(function (d) {
          if (d.kind === "Cell")
            return Math.max(CELL_COLLIDE_R, cellWidth(d) / 2 + 4);
          if (d.kind === "Conditional") return COND_COLLIDE_R;
          if (d.kind === "Branch") return 0;
          return REL_COLLIDE_R;
        }),
      );

    this.simulation.on("tick", function () {
      self.ticked();
      self.updateZoomConstraints();
    });

    this.update(data);
  };

  // Releases a pinned node back into the free simulation.
  GraphInstance.prototype.unpinNode = function (event, d) {
    event.stopPropagation();
    d.fx = null;
    d.fy = null;
    this.simulation.alpha(Math.max(this.simulation.alpha(), 0.3)).restart();
  };

  // Merges `data` into this instance's live nodes/links, preserving existing node
  // positions by id -- ids are only unique within the one Sheet this instance was
  // created for (see `to_graph_data` in `adam-web-ui/src/graph/data.rs`), which is
  // safe here specifically because a *different* Sheet always gets a brand new
  // `GraphInstance` (via the public `init` below) rather than reusing this one.
  /**
   * Reconciles incoming graph data into this live instance.
   * @param {{nodes: Array<object>, links: Array<object>}} data Graph data whose
   * node ids are unique within this instance.
   * @postcondition Existing nodes with matching ids retain object identity,
   * positions, and pin state; value and label fields are refreshed. The
   * simulation restarts only when topology or visibility changes.
   * @complexity O(n + m) time and O(n + m) auxiliary space.
   */
  GraphInstance.prototype.update = function (data) {
    var self = this;
    this.latestData = data;
    if (!this.svg) return;

    // Empty graphs can later gain nodes without becoming a fresh lifecycle.
    var isFirstPopulation = !this.initialFitRequested;

    var reconciled = reconcileNodes(
      { nodes: this.nodes, links: this.links },
      data,
    );
    var structureChanged = reconciled.structureChanged;
    var relabeledIds = reconciled.relabeledIds;
    var oldNodeMap = new Map(this.nodes.map(function (n) { return [n.id, n]; }));
    this.nodes = reconciled.nodes;
    this.links = reconciled.links;
    var nodeMap = new Map(
      this.nodes.map(function (n) {
        return [n.id, n];
      }),
    );

    var changedSet = new Set(data.changed || []);

    var controlledIds = new Set();
    var activeIds = new Set();
    this.links.forEach(function (l) {
      if (l.kind !== "Control") return;
      var tgtId = typeof l.target === "object" ? l.target.id : l.target;
      controlledIds.add(tgtId);
      if (l.branch_active) activeIds.add(tgtId);
    });
    function isInactive(id) {
      return controlledIds.has(id) && !activeIds.has(id);
    }

    var newHiddenIds = this.showInactive
      ? new Set()
      : new Set(
          this.nodes
            .filter(function (n) {
              return isInactive(n.id);
            })
            .map(function (n) {
              return n.id;
            }),
        );
    var hiddenSetChanged = !setsEqual(newHiddenIds, this.hiddenNodeIds);
    this.hiddenNodeIds = newHiddenIds;
    structureChanged = structureChanged || hiddenSetChanged;

    function touchesHidden(l) {
      var srcId = typeof l.source === "object" ? l.source.id : l.source;
      var tgtId = typeof l.target === "object" ? l.target.id : l.target;
      return self.hiddenNodeIds.has(srcId) || self.hiddenNodeIds.has(tgtId);
    }
    var visibleNodes = this.nodes.filter(function (n) {
      return !self.hiddenNodeIds.has(n.id);
    });
    var visibleLinks = this.links.filter(function (l) {
      return !touchesHidden(l);
    });

    var cellNodes = visibleNodes.filter(function (n) {
      return n.kind === "Cell";
    });
    var relNodes = visibleNodes.filter(function (n) {
      return n.kind === "Relationship";
    });
    var condNodes = visibleNodes.filter(function (n) {
      return n.kind === "Conditional";
    });
    var constraintLinks = visibleLinks.filter(function (l) {
      return l.kind === "Constraint";
    });
    var controlLinks = visibleLinks.filter(function (l) {
      return l.kind === "Control";
    });

    this.linkLayer
      .selectAll("line")
      .data(constraintLinks, function (d) {
        var src = typeof d.source === "object" ? d.source.id : d.source;
        var tgt = typeof d.target === "object" ? d.target.id : d.target;
        return src + "-" + tgt;
      })
      .join("line")
      .attr("class", "link");

    this.controlLinkLayer
      .selectAll("line")
      .data(controlLinks, function (d) {
        var src = typeof d.source === "object" ? d.source.id : d.source;
        var tgt = typeof d.target === "object" ? d.target.id : d.target;
        return src + "-" + tgt;
      })
      .join("line")
      .attr("class", "link-control")
      .attr("stroke-dasharray", "5 3")
      .attr("marker-end", function (d) {
        var tgtId = typeof d.target === "object" ? d.target.id : d.target;
        var tgtNode = nodeMap.get(tgtId);
        return tgtNode && tgtNode.kind === "Branch"
          ? null
          : "url(#" + self.dotMarkerId + ")";
      })
      .style("stroke", function (d) {
        return d.branch_active ? null : INACTIVE_STROKE;
      });

    var labelSel = this.labelLayer
      .selectAll("text")
      .data(cellNodes, function (d) {
        return d.id;
      })
      .join("text")
      .attr("class", "node-label")
      .text(function (d) {
        return d.label;
      });

    var valueSel = this.valueLayer
      .selectAll("text")
      .data(cellNodes, function (d) {
        return d.id;
      })
      .join("text")
      .attr("class", "node-value")
      .text(function (d) {
        return d.value || "";
      });

    labelSel.each(function (d) {
      if (oldNodeMap.has(d.id) && !relabeledIds.has(d.id)) return;
      d.w = Math.max(CELL_W, this.getBBox().width + CELL_LABEL_PADDING);
    });
    valueSel.each(function (d) {
      if (
        oldNodeMap.has(d.id) &&
        !changedSet.has(d.id) &&
        !relabeledIds.has(d.id)
      )
        return;
      d.w = Math.max(d.w, this.getBBox().width + CELL_LABEL_PADDING);
    });

    this.cellLayer
      .selectAll("rect")
      .data(cellNodes, function (d) {
        return d.id;
      })
      .join("rect")
      .attr("class", "node-cell")
      .attr("width", cellWidth)
      .attr("height", CELL_H)
      .attr("rx", CELL_RX)
      .call(dragBehavior(this.simulation))
      .on("dblclick", function (event, d) {
        self.unpinNode(event, d);
      });

    this.relLayer
      .selectAll("circle")
      .data(relNodes, function (d) {
        return d.id;
      })
      .join("circle")
      .attr("class", "node-relationship")
      .attr("r", REL_R)
      .call(dragBehavior(this.simulation))
      .on("dblclick", function (event, d) {
        self.unpinNode(event, d);
      });

    (function () {
      self.relLayer.selectAll("circle").style("stroke", function (d) {
        return isInactive(d.id) ? INACTIVE_STROKE : null;
      });
      self.linkLayer
        .selectAll("line")
        .style("stroke", function (d) {
          var srcId = typeof d.source === "object" ? d.source.id : d.source;
          var tgtId = typeof d.target === "object" ? d.target.id : d.target;
          return isInactive(srcId) || isInactive(tgtId)
            ? INACTIVE_STROKE
            : null;
        })
        .attr("marker-end", function (d) {
          if (!d.directed) return null;
          var srcId = typeof d.source === "object" ? d.source.id : d.source;
          var tgtId = typeof d.target === "object" ? d.target.id : d.target;
          if (isInactive(srcId) || isInactive(tgtId)) return null;
          var tgtNode = nodeMap.get(tgtId);
          return tgtNode ? "url(#" + self.arrowMarkerId + ")" : null;
        });
    })();

    (function () {
      var forcedSet = new Set(data.forced || []);
      var forcedRelSet = new Set(data.forced_relationships || []);
      self.cellLayer.selectAll("rect").classed("forced", function (d) {
        return forcedSet.has(d.id);
      });
      self.relLayer.selectAll("circle").classed("forced", function (d) {
        return forcedRelSet.has(d.id);
      });
      self.linkLayer.selectAll("line").classed("forced-edge", function (d) {
        var srcId = typeof d.source === "object" ? d.source.id : d.source;
        var tgtId = typeof d.target === "object" ? d.target.id : d.target;
        return (
          forcedSet.has(srcId) ||
          forcedSet.has(tgtId) ||
          forcedRelSet.has(srcId) ||
          forcedRelSet.has(tgtId)
        );
      });
    })();

    this.condLayer
      .selectAll("rect")
      .data(condNodes, function (d) {
        return d.id;
      })
      .join("rect")
      .attr("class", "node-conditional")
      .attr("width", COND_SIZE * 2)
      .attr("height", COND_SIZE * 2)
      .call(dragBehavior(this.simulation))
      .on("dblclick", function (event, d) {
        self.unpinNode(event, d);
      });

    if (changedSet.size > 0) {
      this.cellLayer
        .selectAll("rect")
        .filter(function (d) {
          return changedSet.has(d.id);
        })
        .transition()
        .duration(PULSE_ON_MS)
        .style("fill", PULSE_COLOR)
        .transition()
        .duration(PULSE_OFF_MS)
        .style("fill", null);
    }

    this.simulation.nodes(visibleNodes);
    this.simulation.force("link").links(visibleLinks);

    if (isFirstPopulation) {
      // Nothing to animate from -- settle synchronously and snap the view to fit.
      this.settleSimulation(true);
    } else if (structureChanged) {
      this.ticked();
      this.updateZoomConstraints();
      this.simulation.alpha(1).restart();
    } else {
      this.ticked();
      this.updateZoomConstraints();
    }
  };

  GraphInstance.prototype.ticked = function () {
    this.linkLayer.selectAll("line").each(function (d) {
      var ep = linkEndpoints(d);
      d3.select(this)
        .attr("x1", ep.x1)
        .attr("y1", ep.y1)
        .attr("x2", ep.x2)
        .attr("y2", ep.y2);
    });

    this.controlLinkLayer.selectAll("line").each(function (d) {
      var ep = linkEndpoints(d);
      var t = d.target;
      var tgtR =
        t.kind === "Branch"
          ? 0
          : (t.kind === "Conditional" ? COND_COLLIDE_R : REL_R) +
            NODE_STROKE_WIDTH / 2 +
            CONTROL_DOT_RADIUS;
      var tgtPt = circleEdgePoint(d.source.x, d.source.y, t.x, t.y, tgtR);
      d3.select(this)
        .attr("x1", ep.x1)
        .attr("y1", ep.y1)
        .attr("x2", tgtPt.x)
        .attr("y2", tgtPt.y);
    });

    this.cellLayer
      .selectAll("rect")
      .attr("x", function (d) {
        return d.x - cellWidth(d) / 2;
      })
      .attr("y", function (d) {
        return d.y - CELL_H / 2;
      });

    this.relLayer
      .selectAll("circle")
      .attr("cx", function (d) {
        return d.x;
      })
      .attr("cy", function (d) {
        return d.y;
      });

    this.condLayer.selectAll("rect").attr("transform", function (d) {
      return (
        "translate(" +
        d.x +
        "," +
        d.y +
        ") rotate(45) translate(" +
        -COND_SIZE +
        "," +
        -COND_SIZE +
        ")"
      );
    });

    this.labelLayer
      .selectAll("text")
      .attr("x", function (d) {
        return d.x;
      })
      .attr("y", function (d) {
        return d.y - 4;
      });

    this.valueLayer
      .selectAll("text")
      .attr("x", function (d) {
        return d.x;
      })
      .attr("y", function (d) {
        return d.y + 10;
      });
  };

  /** Proposes an animated zoom increase; only model-approved frames render. */
  GraphInstance.prototype.zoomIn = function () {
    if (!this.svg || !this.zoom) return;
    if (this.resetPending) this.resetPending = "cancelled";
    this.svg.transition().duration(200).call(this.zoom.scaleBy, 1.3);
  };

  /** Proposes an animated zoom decrease using model-provided gesture hints. */
  GraphInstance.prototype.zoomOut = function () {
    if (!this.svg || !this.zoom) return;
    if (this.resetPending) this.resetPending = "cancelled";
    this.svg
      .transition()
      .duration(200)
      .call(this.zoom.scaleBy, 1 / 1.3);
  };

  /**
   * Requests a fresh Fit target before animating; an initial Fit displays immediately.
   * A user gesture or another zoom button cancels a waiting reset target.
   */
  GraphInstance.prototype.resetZoom = function () {
    if (!this.svg || !this.zoom) return;
    this.svg.interrupt();
    this.resetPending = this.approved ? "waiting" : null;
    this.submitAction({ kind: "fit" });
  };

  /**
   * Interpolates toward a freshly approved Fit through approved proposals.
   * A completed transition restores Fit aspiration; interruption leaves the
   * user's newer proposal intact.
   * @param {{x: number, y: number, k: number}} fit Model-approved target.
   */
  GraphInstance.prototype.animateFit = function (fit) {
    var self = this;
    this.resetAnimating = true;
    this.svg
      .transition()
      .duration(300)
      .call(this.zoom.transform, d3.zoomIdentity.translate(fit.x, fit.y).scale(fit.k))
      .on("end.fit", function () {
        self.resetAnimating = false;
        self.submitAction({ kind: "fit" });
      })
      .on("interrupt.fit cancel.fit", function () { self.resetAnimating = false; });
  };

  GraphInstance.prototype.setShowInactive = function (value) {
    this.showInactive = value;
    if (this.svg) this.update(this.latestData);
  };

  /**
   * Retires the generation/subscription before stopping observers and D3 resources.
   * Late replies, transition events, and observer callbacks cannot render again.
   * @complexity O(n + m) in removed SVG nodes and links.
   */
  GraphInstance.prototype.destroy = function () {
    this.queue.dispose();
    this.unsubscribe();
    if (this.resizeObserver) {
      this.resizeObserver.disconnect();
      this.resizeObserver = null;
    }
    if (this.simulation) {
      this.simulation.stop();
      this.simulation = null;
    }
    if (this.svg) {
      this.svg.interrupt();
      this.svg.remove();
      this.svg = null;
    }
  };

  // ---- Public registry: window.beginGraph, keyed by container id ----

  function init(id, data) {
    var existing = instances.get(id);
    if (existing) existing.destroy();
    var channel = root.__beginGraphChannels && root.__beginGraphChannels[id];
    if (!channel) throw new Error("GraphView approval channel is not installed for " + id);
    var inst = new GraphInstance(id, channel);
    instances.set(id, inst);
    inst.start(data);
  }

  function update(id, data) {
    var inst = instances.get(id);
    if (inst) inst.update(data);
  }

  function destroy(id) {
    var inst = instances.get(id);
    if (inst) {
      inst.destroy();
      instances.delete(id);
    }
  }

  function zoomIn(id) {
    var inst = instances.get(id);
    if (inst) inst.zoomIn();
  }
  function zoomOut(id) {
    var inst = instances.get(id);
    if (inst) inst.zoomOut();
  }
  function resetZoom(id) {
    var inst = instances.get(id);
    if (inst) inst.resetZoom();
  }
  function setShowInactive(id, value) {
    var inst = instances.get(id);
    if (inst) inst.setShowInactive(value);
  }

  if (typeof window !== "undefined") {
    root.beginGraph = {
      init: init,
      update: update,
      destroy: destroy,
      zoomIn: zoomIn,
      zoomOut: zoomOut,
      resetZoom: resetZoom,
      setShowInactive: setShowInactive,
      createChannel: createChannel,
    };
  }

  if (typeof module !== "undefined" && module.exports) {
    module.exports = {
      cellEdgePoint: cellEdgePoint,
      circleEdgePoint: circleEdgePoint,
      linkEndpoints: linkEndpoints,
      computeBBox: computeBBox,
      reconcileNodes: reconcileNodes,
      ProposalQueue: ProposalQueue,
      nextCounter: nextCounter,
      createChannel: createChannel,
      GraphInstance: GraphInstance,
    };
  }
})();
