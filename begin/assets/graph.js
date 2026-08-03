(function () {
    // Tunable layout constants
    var LINK_DISTANCE = 80;
    var CHARGE_STRENGTH = -300;
    var CELL_W = 60;
    var CELL_H = 36;
    var CELL_RX = 4;
    var CELL_LABEL_PADDING = 16;                          // horizontal padding added around the wider of the label/value text when sizing a cell's box
    var REL_R = 16;
    var COND_SIZE = 20;                                   // NEW: diamond half-width/height
    var CELL_COLLIDE_R = 38;
    var REL_COLLIDE_R = 22;
    var COND_COLLIDE_R = COND_SIZE * Math.SQRT2;          // NEW: diamond circumradius
    var NODE_STROKE_WIDTH = 1.5;                          // NEW: matches .node-cell/-relationship/-conditional stroke-width
    var CONTROL_DOT_RADIUS = 2.4;                         // NEW: rendered radius of the '#dot' marker (r=4 in a 10-wide viewBox scaled to a 6-wide marker: 4 * 6/10)
    var FIT_MARGIN = 16;                                  // extra breathing room around node bounds (stroke width, labels) so Fit doesn't clip geometry
    var PULSE_COLOR = '#f90';
    var PULSE_ON_MS = 200;
    var PULSE_OFF_MS = 400;
    var INACTIVE_STROKE = '#ccc';                         // NEW: stroke color for inactive elements

    var svg = null;
    var simulation = null;
    var controlLinkLayer = null;                          // NEW
    var linkLayer = null;
    var cellLayer = null;
    var relLayer = null;
    var condLayer = null;                                 // NEW
    var labelLayer = null;
    var valueLayer = null;
    var nodes = [];
    var links = [];
    var width = 800;
    var height = 600;
    var resizeObserver = null;
    var zoom = null;
    var zoomLayer = null;
    var hasInitialFit = false;
    var MAX_ZOOM = 8;
    var latestData = null;
    var currentSourceId = null;                          // which demo/file `nodes`/`links` currently belong to

    // Returns the point on the rect boundary of a cell centered at (tx,ty)
    // along the approach line from (sx,sy) to (tx,ty). `hw`/`hh` are that
    // cell's own half-width/half-height (each cell can have a different
    // width - see cellWidth()), defaulting to the base cell size.
    function cellEdgePoint(sx, sy, tx, ty, hw, hh) {
        if (hw === undefined) hw = CELL_W / 2;
        if (hh === undefined) hh = CELL_H / 2;
        var dx = tx - sx, dy = ty - sy;
        var dist = Math.sqrt(dx * dx + dy * dy);
        if (dist < 1) return { x: tx, y: ty };
        var nx = dx / dist, ny = dy / dist;
        var td = Math.abs(nx) > 1e-9 ? hw / Math.abs(nx) : Infinity;
        var ld = Math.abs(ny) > 1e-9 ? hh / Math.abs(ny) : Infinity;
        var d = Math.min(td, ld);
        return { x: tx - nx * d, y: ty - ny * d };
    }

    // Returns cell node `d`'s rendered box width, falling back to the base
    // cell width before it's been measured (see the "Size cell boxes to fit
    // their label/value text" step in update()).
    function cellWidth(d) {
        return d.w || CELL_W;
    }

    // Returns the point on the boundary of a circle (centered at cx,cy, radius r)
    // along the approach line from (sx,sy) to (cx,cy).
    function circleEdgePoint(sx, sy, cx, cy, r) {
        var dx = cx - sx, dy = cy - sy;
        var dist = Math.sqrt(dx * dx + dy * dy);
        if (dist < 1) return { x: cx, y: cy };
        return { x: cx - dx / dist * r, y: cy - dy / dist * r };
    }

    // CHANGED: handles Cell, Relationship, Conditional, and Branch source/target kinds.
    function linkEndpoints(d) {
        var s = d.source, t = d.target;
        function edgePt(node, ox, oy) {
            if (node.kind === 'Cell') return cellEdgePoint(ox, oy, node.x, node.y, cellWidth(node) / 2, CELL_H / 2);
            if (node.kind === 'Branch') return { x: node.x, y: node.y };
            var r = node.kind === 'Conditional' ? COND_COLLIDE_R : REL_R;
            return circleEdgePoint(ox, oy, node.x, node.y, r);
        }
        var srcPt = edgePt(s, t.x, t.y);
        var tgtPt = edgePt(t, s.x, s.y);
        return { x1: srcPt.x, y1: srcPt.y, x2: tgtPt.x, y2: tgtPt.y };
    }

    // Returns the axis-aligned bounding box of all node visuals, in graph
    // (pre-zoom-transform) coordinates. Falls back to the viewport when there
    // are no nodes yet.
    function computeBBox() {
        if (nodes.length === 0) {
            return { minX: 0, minY: 0, maxX: width, maxY: height };
        }
        var minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
        nodes.forEach(function (n) {
            var hw, hh;
            if (n.kind === 'Cell') { hw = cellWidth(n) / 2; hh = CELL_H / 2; }
            else if (n.kind === 'Conditional') { hw = COND_COLLIDE_R; hh = COND_COLLIDE_R; }
            else if (n.kind === 'Branch') { hw = 0; hh = 0; }
            else { hw = REL_R; hh = REL_R; }
            minX = Math.min(minX, n.x - hw);
            minY = Math.min(minY, n.y - hh);
            maxX = Math.max(maxX, n.x + hw);
            maxY = Math.max(maxY, n.y + hh);
        });
        return {
            minX: minX - FIT_MARGIN, minY: minY - FIT_MARGIN,
            maxX: maxX + FIT_MARGIN, maxY: maxY + FIT_MARGIN
        };
    }

    // Returns the scale that fits `bbox` entirely inside the current
    // viewport, and the centered zoom transform at that scale.
    function fitTransformFor(bbox) {
        var cx = (bbox.minX + bbox.maxX) / 2;
        var cy = (bbox.minY + bbox.maxY) / 2;
        var contentW = Math.max(bbox.maxX - bbox.minX, 1);
        var contentH = Math.max(bbox.maxY - bbox.minY, 1);
        var fitScale = Math.min(width / contentW, height / contentH);
        return {
            fitScale: fitScale,
            transform: d3.zoomIdentity.translate(width / 2, height / 2).scale(fitScale).translate(-cx, -cy)
        };
    }

    // Recomputes zoom scale/pan bounds from the current node layout. On the
    // first call after init(), or whenever `forceFit` is true, snaps the
    // view to fit; otherwise preserves the user's current pan/zoom, only
    // re-clamping it if it now falls outside the new bounds.
    function updateZoomConstraints(forceFit) {
        var bbox = computeBBox();
        var fit = fitTransformFor(bbox);
        var maxScale = Math.max(fit.fitScale, MAX_ZOOM);
        var extent = [[0, 0], [width, height]];
        var translateExtent = [[bbox.minX, bbox.minY], [bbox.maxX, bbox.maxY]];
        zoom.scaleExtent([fit.fitScale, maxScale])
            .translateExtent(translateExtent)
            .extent(extent);
        if (!hasInitialFit || forceFit) {
            svg.call(zoom.transform, fit.transform);
            hasInitialFit = true;
        } else {
            // zoom.transform() only runs d3's clamping logic when passed a
            // function, not a plain transform object — so explicitly clamp
            // the preserved transform before applying it, otherwise a
            // shrunk translateExtent/scaleExtent would never actually pull
            // an out-of-bounds view back in. d3's own constrain function
            // (exposed through the public zoom.constrain() accessor) only
            // adjusts x/y against translateExtent — it leaves k untouched —
            // so clamp k against scaleExtent ourselves first.
            var current = d3.zoomTransform(svg.node());
            var clampedK = Math.max(fit.fitScale, Math.min(maxScale, current.k));
            var rescaled = current.scale(clampedK / current.k);
            var clamped = zoom.constrain()(rescaled, extent, translateExtent);
            svg.call(zoom.transform, clamped);
        }
    }

    // Runs the simulation synchronously until settled, then updates the display.
    // `forceFit` is passed straight through to updateZoomConstraints() - see
    // its doc comment.
    function settleSimulation(forceFit) {
        var n = Math.ceil(Math.log(simulation.alphaMin()) / Math.log(1 - simulation.alphaDecay()));
        simulation.stop().alpha(1).tick(n);
        ticked();
        updateZoomConstraints(forceFit);
    }

    function init(containerId, data, sourceId) {
        // Tear down any previous init (component remount / hot-reload).
        if (resizeObserver) { resizeObserver.disconnect(); resizeObserver = null; }
        if (simulation) { simulation.stop(); simulation = null; }
        if (svg) { svg.remove(); svg = null; }
        zoom = null;
        zoomLayer = null;
        hasInitialFit = false;
        nodes = [];
        links = [];
        latestData = data;
        currentSourceId = sourceId;

        var container = document.getElementById(containerId);

        // Keep observing for the life of this mount (torn down above on the
        // next init() call) so the view area tracks the container's size
        // continuously, not just once at mount. The first firing measures
        // after layout has settled — a plain clientWidth/clientHeight read
        // here can race layout and return a stale (often zero) size, which
        // is what made the graph appear cut off on first load — and builds
        // the graph; every later firing just resizes the existing canvas.
        //
        // ResizeObserver's first callback fires asynchronously, not on this
        // observe() call, so any update() in between is a no-op (svg is
        // still null) and must not be lost — buildGraph reads latestData
        // (kept current by update()) rather than this closure's `data`.
        resizeObserver = new ResizeObserver(function () {
            width = container.clientWidth || width;
            height = container.clientHeight || height;
            if (!svg) {
                buildGraph(container, latestData);
            } else {
                resizeCanvas();
            }
        });
        resizeObserver.observe(container);
    }

    // Resizes the existing SVG to the current width/height without touching
    // node positions or restarting the simulation. Keeps viewBox equal to the
    // pixel size (not just fixed) so the browser never stretches existing
    // content to fill the new size — that mismatch is what caused the graph
    // to visually distort on resize before pan/zoom existed. Recomputing the
    // zoom constraints preserves the user's current pan/zoom, only
    // re-clamping it if it now falls outside the new bounds.
    function resizeCanvas() {
        svg.attr('width', width)
            .attr('height', height)
            .attr('viewBox', [0, 0, width, height]);
        simulation.force('center').x(width / 2).y(height / 2);
        updateZoomConstraints();
    }

    function buildGraph(container, data) {
        svg = d3.select(container)
            .append('svg')
            .attr('width', width)
            .attr('height', height)
            .attr('viewBox', [0, 0, width, height]);

        var defs = svg.append('defs');

        // Arrowhead: refX=10 places the tip (at local x=10) at the line endpoint.
        // Lines are drawn edge-to-edge so the tip lands exactly at the node boundary.
        defs.append('marker')
            .attr('id', 'arrowhead')
            .attr('viewBox', '0 -5 10 10')
            .attr('refX', 10)
            .attr('refY', 0)
            .attr('markerWidth', 8)
            .attr('markerHeight', 8)
            .attr('markerUnits', 'userSpaceOnUse')
            .attr('orient', 'auto')
            // context-stroke: inherit the referencing line's current stroke (which
            // switches between the enabled/disabled colors set on the line itself)
            // so the arrowhead always matches its edge without duplicating that logic.
            .append('path').attr('d', 'M0,-5L10,0L0,5').attr('fill', 'context-stroke');

        // Dot marker: caps control links where they meet the relationship they target.
        defs.append('marker')
            .attr('id', 'dot')
            .attr('viewBox', '0 0 10 10')
            .attr('refX', 5)
            .attr('refY', 5)
            .attr('markerWidth', 6)
            .attr('markerHeight', 6)
            .attr('markerUnits', 'userSpaceOnUse')
            .attr('orient', 'auto')
            .append('circle').attr('cx', 5).attr('cy', 5).attr('r', 4).attr('fill', 'context-stroke');

        // Layer z-order: bg → control links → constraint links → cells → rels → conditionals → labels → values
        zoomLayer = svg.append('g').attr('class', 'zoom-layer');
        zoomLayer.append('g').attr('class', 'bg-layer');
        controlLinkLayer = zoomLayer.append('g').attr('class', 'control-link-layer'); // NEW
        linkLayer = zoomLayer.append('g').attr('class', 'link-layer');
        cellLayer = zoomLayer.append('g').attr('class', 'cell-layer');
        relLayer = zoomLayer.append('g').attr('class', 'rel-layer');
        condLayer = zoomLayer.append('g').attr('class', 'cond-layer');               // NEW
        labelLayer = zoomLayer.append('g').attr('class', 'label-layer');
        valueLayer = zoomLayer.append('g').attr('class', 'value-layer');

        // Pan/zoom: the transform is applied to zoomLayer; scale/pan bounds
        // are set by updateZoomConstraints() once node positions are known.
        zoom = d3.zoom().on('zoom', function (event) {
            zoomLayer.attr('transform', event.transform);
        });
        svg.call(zoom);

        simulation = d3.forceSimulation()
            .force('link', d3.forceLink().id(function (d) { return d.id; }).distance(function (d) {
                var sKind = typeof d.source === 'object' ? d.source.kind : null;
                var tKind = typeof d.target === 'object' ? d.target.kind : null;
                return (sKind === 'Branch' || tKind === 'Branch') ? LINK_DISTANCE / 2 : LINK_DISTANCE;
            }))
            .force('charge', d3.forceManyBody().strength(CHARGE_STRENGTH))
            .force('center', d3.forceCenter(width / 2, height / 2))
            // CHANGED: collision radius handles Conditional nodes.
            .force('collide', d3.forceCollide().radius(function (d) {
                if (d.kind === 'Cell') return Math.max(CELL_COLLIDE_R, cellWidth(d) / 2 + 4);
                if (d.kind === 'Conditional') return COND_COLLIDE_R;
                if (d.kind === 'Branch') return 0;
                return REL_COLLIDE_R;
            }));

        // The live simulation timer is the only driver of continuous position
        // changes outside an explicit settleSimulation()/update() call (see
        // updateZoomConstraints's call site below), so it's the only place
        // that needs to recompute zoom bounds on every frame.
        simulation.on('tick', function () {
            ticked();
            updateZoomConstraints();
        });

        // currentSourceId, not a fresh parameter: init() already set it to the
        // correct value before this (possibly-async, via ResizeObserver's
        // first firing) call, so passing it straight through here can't
        // diverge the way an `undefined` argument would.
        update(data, currentSourceId);
    }

    // Returns a d3.drag() behavior that pins a node's position while it's
    // being dragged and reheats `sim` so the rest of the graph reacts live.
    // Deliberately does NOT clear fx/fy on drag-end — the node stays exactly
    // where it was dropped; see unpinNode() for how a node is released.
    function dragBehavior(sim) {
        return d3.drag()
            .on('start', function (event, d) {
                // Both d3.zoom (on the <svg>) and this drag (on the node
                // shape) listen for pointer-down; without this the same
                // gesture would also pan the canvas.
                event.sourceEvent.stopPropagation();
            })
            .on('drag', function (event, d) {
                // 'start'/'end' fire on every pointerdown/pointerup, even a
                // plain click with no movement, but 'drag' only fires once
                // actual movement occurs — so gate the reheat and cursor
                // state on it (via the 'dragging' class, set at most once
                // per gesture here) to keep a no-movement click a true no-op.
                if (!d3.select(this).classed('dragging')) {
                    sim.alphaTarget(0.3).restart();
                    d3.select(this).classed('dragging', true);
                }
                d.fx = event.x;
                d.fy = event.y;
            })
            .on('end', function (event, d) {
                if (!event.active) sim.alphaTarget(0);
                d3.select(this).classed('dragging', false);
            });
    }

    // Releases a pinned node back into the free simulation.
    function unpinNode(event, d) {
        event.stopPropagation();
        d.fx = null;
        d.fy = null;
        simulation.alpha(Math.max(simulation.alpha(), 0.3)).restart();
    }

    function update(data, sourceId) {
        latestData = data;
        // Guard: no-op if not yet initialized
        if (!svg) return;

        // A different demo/file just became active: wipe the node/link cache
        // entirely rather than let the id-based merge below run against it.
        // Node ids are only unique within one Sheet (see cell_node_id() in
        // bridge.rs), so without this, switching sources could silently
        // recycle an old id for an unrelated cell and inherit its stale
        // layout position — the same root cause the relabeledIds check below
        // guards against for stale box widths, generalized to every other
        // per-node field a stale reused object could carry over.
        if (sourceId !== currentSourceId) {
            nodes = [];
            links = [];
            currentSourceId = sourceId;
        }

        // Detect structural changes before mutating node/link arrays. Link identity
        // is an unordered node-id pair: replanning can flip which end of an existing
        // edge is the source and which is the target (e.g. a relationship's input
        // becomes its output) without changing the graph's topology, and such flips
        // must not be treated as a structural change — doing so forces a full
        // simulation restart (see settleSimulation) that visibly repositions every
        // node, not just the ones on the flipped edge.
        function linkKey(a, b) { return a < b ? a + '|' + b : b + '|' + a; }
        var oldNodeIds = new Set(nodes.map(function (n) { return n.id; }));
        var oldLinkSet = new Set(links.map(function (l) {
            var src = typeof l.source === 'object' ? l.source.id : l.source;
            var tgt = typeof l.target === 'object' ? l.target.id : l.target;
            return linkKey(src, tgt);
        }));
        var structureChanged = nodes.length !== data.nodes.length
            || links.length !== data.links.length
            || data.nodes.some(function (n) { return !oldNodeIds.has(n.id); })
            || data.links.some(function (l) { return !oldLinkSet.has(linkKey(l.source, l.target)); });

        // Preserve existing node positions by merging into incoming data.
        //
        // Node ids are only unique *within* one Sheet: they're built from a
        // cell's raw slotmap index (see cell_node_id() in bridge.rs), and a
        // freshly loaded demo/opened file starts a brand new Sheet whose
        // slotmap indices restart from the same small integers. So switching
        // from one source to an unrelated one can easily reuse an old id for
        // a completely different cell (e.g. the 6th cell allocated in each).
        // relabeledIds tracks exactly that case so the width-measuring step
        // below knows to remeasure even though oldNodeMap already has the id.
        var oldNodeMap = new Map(nodes.map(function (n) { return [n.id, n]; }));
        var relabeledIds = new Set();
        nodes = data.nodes.map(function (n) {
            var existing = oldNodeMap.get(n.id);
            if (existing) {
                if (existing.label !== n.label) relabeledIds.add(n.id);
                existing.kind = n.kind;
                existing.label = n.label;
                existing.value = n.value;
                return existing;
            }
            return Object.assign({}, n);
        });
        var nodeMap = new Map(nodes.map(function (n) { return [n.id, n]; }));
        links = data.links.map(function (l) { return Object.assign({}, l); });

        var changedSet = new Set(data.changed || []);
        var cellNodes = nodes.filter(function (n) { return n.kind === 'Cell'; });
        var relNodes = nodes.filter(function (n) { return n.kind === 'Relationship'; });
        var condNodes = nodes.filter(function (n) { return n.kind === 'Conditional'; }); // NEW
        var constraintLinks = links.filter(function (l) { return l.kind === 'Constraint'; }); // NEW
        var controlLinks = links.filter(function (l) { return l.kind === 'Control'; });         // NEW

        // Constraint links (marker-end and opacity set below in the dimming IIFE)
        linkLayer.selectAll('line')
            .data(constraintLinks, function (d) {         // CHANGED: constraintLinks only
                var src = typeof d.source === 'object' ? d.source.id : d.source;
                var tgt = typeof d.target === 'object' ? d.target.id : d.target;
                return src + '-' + tgt;
            })
            .join('line')
            .attr('class', 'link');

        // NEW: Control links (dashed, dot-capped where they meet their target relationship).
        // Stroke matches the enabled/disabled node stroke color depending on whether
        // this specific branch is currently active.
        controlLinkLayer.selectAll('line')
            .data(controlLinks, function (d) {
                var src = typeof d.source === 'object' ? d.source.id : d.source;
                var tgt = typeof d.target === 'object' ? d.target.id : d.target;
                return src + '-' + tgt;
            })
            .join('line')
            .attr('class', 'link-control')
            .attr('stroke-dasharray', '5 3')
            .attr('marker-end', function (d) {
                var tgtId = typeof d.target === 'object' ? d.target.id : d.target;
                var tgtNode = nodeMap.get(tgtId);
                return (tgtNode && tgtNode.kind === 'Branch') ? null : 'url(#dot)';
            })
            .style('stroke', function (d) { return d.branch_active ? null : INACTIVE_STROKE; });

        // Join cell name labels (centered inside rect)
        var labelSel = labelLayer.selectAll('text')
            .data(cellNodes, function (d) { return d.id; })
            .join('text')
            .attr('class', 'node-label')
            .text(function (d) { return d.label; });

        // Join cell value labels (below the name, inside rect)
        var valueSel = valueLayer.selectAll('text')
            .data(cellNodes, function (d) { return d.id; })
            .join('text')
            .attr('class', 'node-value')
            .text(function (d) { return d.value || ''; });

        // Size cell boxes to fit their label/value text: measure each cell's
        // rendered label and value text (getBBox reflects the actual glyphs
        // and CSS font in effect, so this tracks font/content changes without
        // guessing at character widths), and grow the box to the wider of
        // the two plus padding. Must run after the text joins above (so
        // there's rendered text to measure) and before the rect join below
        // (so rect widths can use the result).
        //
        // getBBox() forces SVG layout, so it's restricted to cells that can
        // actually need remeasuring: a cell's label is fixed at creation (it
        // never changes for an id that already existed, unless relabeledIds
        // says the id was actually recycled for a different cell — see the
        // node-merge step above), so it's only measured once, when the node
        // is new; d.w (set here or on a prior update) persists on the reused
        // node object across updates for every other cell. The value text
        // does change at runtime, so it's remeasured for new/relabeled cells
        // plus whichever existing ones are in `changedSet`, rather than every
        // cell on every update.
        labelSel.each(function (d) {
            if (oldNodeMap.has(d.id) && !relabeledIds.has(d.id)) return;
            d.w = Math.max(CELL_W, this.getBBox().width + CELL_LABEL_PADDING);
        });
        valueSel.each(function (d) {
            if (oldNodeMap.has(d.id) && !changedSet.has(d.id) && !relabeledIds.has(d.id)) return;
            d.w = Math.max(d.w, this.getBBox().width + CELL_LABEL_PADDING);
        });

        // Join cell rects
        cellLayer.selectAll('rect')
            .data(cellNodes, function (d) { return d.id; })
            .join('rect')
            .attr('class', 'node-cell')
            .attr('width', cellWidth)
            .attr('height', CELL_H)
            .attr('rx', CELL_RX)
            .call(dragBehavior(simulation))
            .on('dblclick', unpinNode);

        // Join relationship circles
        relLayer.selectAll('circle')
            .data(relNodes, function (d) { return d.id; })
            .join('circle')
            .attr('class', 'node-relationship')
            .attr('r', REL_R)
            .call(dragBehavior(simulation))
            .on('dblclick', unpinNode);

        // Dim inactive relationship circles and their constraint links.
        // A relationship is inactive if any control link targets it but none are active.
        // Inactive links also lose their arrowheads.
        (function () {
            var controlledRelIds = new Set();
            var activeRelIds = new Set();
            controlLinks.forEach(function (l) {
                var tgtId = typeof l.target === 'object' ? l.target.id : l.target;
                controlledRelIds.add(tgtId);
                if (l.branch_active) activeRelIds.add(tgtId);
            });
            function isInactiveRel(id) {
                return controlledRelIds.has(id) && !activeRelIds.has(id);
            }
            relLayer.selectAll('circle').style('stroke', function (d) {
                return isInactiveRel(d.id) ? INACTIVE_STROKE : null;
            });
            linkLayer.selectAll('line')
                .style('stroke', function (d) {
                    var srcId = typeof d.source === 'object' ? d.source.id : d.source;
                    var tgtId = typeof d.target === 'object' ? d.target.id : d.target;
                    return (isInactiveRel(srcId) || isInactiveRel(tgtId)) ? INACTIVE_STROKE : null;
                })
                .attr('marker-end', function (d) {
                    if (!data.arrows) return null;
                    var srcId = typeof d.source === 'object' ? d.source.id : d.source;
                    var tgtId = typeof d.target === 'object' ? d.target.id : d.target;
                    if (isInactiveRel(srcId) || isInactiveRel(tgtId)) return null;
                    var tgtNode = nodeMap.get(tgtId);
                    return tgtNode ? 'url(#arrowhead)' : null;
                });
        }());

        // Highlight forced cells (see adam_rs::Sheet::is_forced) and forced
        // relationships (see adam_rs::Sheet::is_relationship_forced) — those
        // with only one viable method, regardless of cell strength — plus every
        // constraint edge touching either: the incoming edge into a forced
        // relationship, its outgoing edge(s), and any further edges carrying a
        // forced cell's guaranteed value onward. Both always belong to a currently
        // active relationship, so this never overlaps with the inactive-relationship
        // dimming above.
        (function () {
            var forcedSet = new Set(data.forced || []);
            var forcedRelSet = new Set(data.forced_relationships || []);
            cellLayer.selectAll('rect')
                .classed('forced', function (d) { return forcedSet.has(d.id); });
            relLayer.selectAll('circle')
                .classed('forced', function (d) { return forcedRelSet.has(d.id); });
            linkLayer.selectAll('line')
                .classed('forced-edge', function (d) {
                    var srcId = typeof d.source === 'object' ? d.source.id : d.source;
                    var tgtId = typeof d.target === 'object' ? d.target.id : d.target;
                    return forcedSet.has(srcId) || forcedSet.has(tgtId)
                        || forcedRelSet.has(srcId) || forcedRelSet.has(tgtId);
                });
        }());

        // NEW: Conditional diamond nodes (rotated rect)
        condLayer.selectAll('rect')
            .data(condNodes, function (d) { return d.id; })
            .join('rect')
            .attr('class', 'node-conditional')
            .attr('width', COND_SIZE * 2)
            .attr('height', COND_SIZE * 2)
            .call(dragBehavior(simulation))
            .on('dblclick', unpinNode);

        // Pulse changed cells
        if (changedSet.size > 0) {
            cellLayer.selectAll('rect')
                .filter(function (d) { return changedSet.has(d.id); })
                .transition().duration(PULSE_ON_MS)
                .style('fill', PULSE_COLOR)
                .transition().duration(PULSE_OFF_MS)
                .style('fill', null);
        }

        // Feed ALL links to the simulation (both constraint and control) so D3
        // resolves source/target strings to node objects for ticked().
        simulation.nodes(nodes);
        simulation.force('link').links(links);

        if (structureChanged) {
            // Settle synchronously so the graph is stable before display, and
            // fit the view to it: a structural change means a brand new
            // Sheet (a different demo was picked, or the active demo's
            // source hot-reloaded) with freshly laid-out node positions, so
            // the old pan/zoom has nothing meaningful left to preserve.
            settleSimulation(true);
        } else {
            // Only labels/values changed — node positions are unchanged.
            ticked();
        }
    }

    function ticked() {
        // Constraint links: edge-to-edge so arrowheads land at node boundaries.
        linkLayer.selectAll('line').each(function (d) {
            var ep = linkEndpoints(d);
            d3.select(this)
                .attr('x1', ep.x1).attr('y1', ep.y1)
                .attr('x2', ep.x2).attr('y2', ep.y2);
        });

        // NEW: Control links: edge-to-edge so the dot marker just touches the
        // relationship's rendered boundary instead of overlapping it. The target
        // radius is padded by half the node's stroke width (the nominal radius sits
        // on the stroke's centerline, not its outer edge) plus the dot marker's own
        // rendered radius (the marker is centered on the line's endpoint, so without
        // this the dot would straddle the boundary rather than sit tangent to it).
        controlLinkLayer.selectAll('line').each(function (d) {
            var ep = linkEndpoints(d);
            var t = d.target;
            var tgtR = t.kind === 'Branch'
                ? 0
                : (t.kind === 'Conditional' ? COND_COLLIDE_R : REL_R) + NODE_STROKE_WIDTH / 2 + CONTROL_DOT_RADIUS;
            var tgtPt = circleEdgePoint(d.source.x, d.source.y, t.x, t.y, tgtR);
            d3.select(this)
                .attr('x1', ep.x1).attr('y1', ep.y1)
                .attr('x2', tgtPt.x).attr('y2', tgtPt.y);
        });

        cellLayer.selectAll('rect')
            .attr('x', function (d) { return d.x - cellWidth(d) / 2; })
            .attr('y', function (d) { return d.y - CELL_H / 2; });

        relLayer.selectAll('circle')
            .attr('cx', function (d) { return d.x; })
            .attr('cy', function (d) { return d.y; });

        // NEW: Conditional diamond: rotated rect centered at (d.x, d.y).
        condLayer.selectAll('rect')
            .attr('transform', function (d) {
                return 'translate(' + d.x + ',' + d.y + ') rotate(45) translate(' + (-COND_SIZE) + ',' + (-COND_SIZE) + ')';
            });

        // Cell name: upper half of rect
        labelLayer.selectAll('text')
            .attr('x', function (d) { return d.x; })
            .attr('y', function (d) { return d.y - 4; });

        // Cell value: lower half of rect
        valueLayer.selectAll('text')
            .attr('x', function (d) { return d.x; })
            .attr('y', function (d) { return d.y + 10; });
    }

    // Called by the on-screen zoom controls in graph_view.rs.
    function zoomIn() {
        if (!svg || !zoom) return;
        svg.transition().duration(200).call(zoom.scaleBy, 1.3);
    }

    function zoomOut() {
        if (!svg || !zoom) return;
        svg.transition().duration(200).call(zoom.scaleBy, 1 / 1.3);
    }

    function resetZoom() {
        if (!svg || !zoom) return;
        var fit = fitTransformFor(computeBBox());
        svg.transition().duration(300).call(zoom.transform, fit.transform);
    }

    window.beginGraph = { init: init, update: update, zoomIn: zoomIn, zoomOut: zoomOut, resetZoom: resetZoom };
}());
