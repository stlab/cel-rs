(function () {
    // Tunable layout constants
    var LINK_DISTANCE = 80;
    var CHARGE_STRENGTH = -300;
    var CELL_W = 60;
    var CELL_H = 36;
    var CELL_RX = 4;
    var REL_R = 16;
    var COND_SIZE = 20;                                   // NEW: diamond half-width/height
    var CELL_COLLIDE_R = 38;
    var REL_COLLIDE_R = 22;
    var COND_COLLIDE_R = COND_SIZE * Math.SQRT2;          // NEW: diamond circumradius
    var PULSE_COLOR = '#f90';
    var PULSE_ON_MS = 200;
    var PULSE_OFF_MS = 400;
    var BRANCH_COLORS = ['#4a90d9', '#e67e22'];           // NEW: branch 0=blue, 1=orange
    var BRANCH_COLORS_DIM = ['#a8c8f0', '#f5c8a0'];      // NEW: inactive branch colors
    var DEFAULT_BRANCH_COLOR = '#888';                    // NEW: default/no-branch control links
    var DEFAULT_BRANCH_DIM = '#bbb';                      // NEW: inactive default control links
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

    // Returns the point on the rect boundary of a cell centered at (tx,ty)
    // along the approach line from (sx,sy) to (tx,ty).
    function cellEdgePoint(sx, sy, tx, ty) {
        var dx = tx - sx, dy = ty - sy;
        var dist = Math.sqrt(dx * dx + dy * dy);
        if (dist < 1) return { x: tx, y: ty };
        var nx = dx / dist, ny = dy / dist;
        var hw = CELL_W / 2, hh = CELL_H / 2;
        var td = Math.abs(nx) > 1e-9 ? hw / Math.abs(nx) : Infinity;
        var ld = Math.abs(ny) > 1e-9 ? hh / Math.abs(ny) : Infinity;
        var d = Math.min(td, ld);
        return { x: tx - nx * d, y: ty - ny * d };
    }

    // Returns the point on the boundary of a circle (centered at cx,cy, radius r)
    // along the approach line from (sx,sy) to (cx,cy).
    function circleEdgePoint(sx, sy, cx, cy, r) {
        var dx = cx - sx, dy = cy - sy;
        var dist = Math.sqrt(dx * dx + dy * dy);
        if (dist < 1) return { x: cx, y: cy };
        return { x: cx - dx / dist * r, y: cy - dy / dist * r };
    }

    // CHANGED: handles Cell, Relationship, and Conditional source/target kinds.
    function linkEndpoints(d) {
        var s = d.source, t = d.target;
        function edgePt(node, ox, oy) {
            if (node.kind === 'Cell') return cellEdgePoint(ox, oy, node.x, node.y);
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
            if (n.kind === 'Cell') { hw = CELL_W / 2; hh = CELL_H / 2; }
            else if (n.kind === 'Conditional') { hw = COND_COLLIDE_R; hh = COND_COLLIDE_R; }
            else { hw = REL_R; hh = REL_R; }
            minX = Math.min(minX, n.x - hw);
            minY = Math.min(minY, n.y - hh);
            maxX = Math.max(maxX, n.x + hw);
            maxY = Math.max(maxY, n.y + hh);
        });
        return { minX: minX, minY: minY, maxX: maxX, maxY: maxY };
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
    // first call after init(), snaps the view to fit; afterward, preserves
    // the user's current pan/zoom, only re-clamping it if it now falls
    // outside the new bounds.
    function updateZoomConstraints() {
        var bbox = computeBBox();
        var fit = fitTransformFor(bbox);
        var maxScale = Math.max(fit.fitScale, MAX_ZOOM);
        var extent = [[0, 0], [width, height]];
        var translateExtent = [[bbox.minX, bbox.minY], [bbox.maxX, bbox.maxY]];
        zoom.scaleExtent([fit.fitScale, maxScale])
            .translateExtent(translateExtent)
            .extent(extent);
        if (!hasInitialFit) {
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
    function settleSimulation() {
        var n = Math.ceil(Math.log(simulation.alphaMin()) / Math.log(1 - simulation.alphaDecay()));
        simulation.stop().alpha(1).tick(n);
        ticked();
        updateZoomConstraints();
    }

    function init(containerId, data) {
        // Tear down any previous init (component remount / hot-reload).
        if (resizeObserver) { resizeObserver.disconnect(); resizeObserver = null; }
        if (simulation) { simulation.stop(); simulation = null; }
        if (svg) { svg.remove(); svg = null; }
        zoom = null;
        zoomLayer = null;
        hasInitialFit = false;
        nodes = [];
        links = [];

        var container = document.getElementById(containerId);

        // Measure once, after layout has settled, then never resize the graph
        // again — a plain clientWidth/clientHeight read here can race layout
        // and return a stale (often zero) size, which is what made the graph
        // appear cut off on first load.
        resizeObserver = new ResizeObserver(function () {
            resizeObserver.disconnect();
            resizeObserver = null;
            width = container.clientWidth || width;
            height = container.clientHeight || height;
            buildGraph(container, data);
        });
        resizeObserver.observe(container);
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
            .append('path').attr('d', 'M0,-5L10,0L0,5').attr('fill', '#999');

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
            .force('link', d3.forceLink().id(function (d) { return d.id; }).distance(LINK_DISTANCE))
            .force('charge', d3.forceManyBody().strength(CHARGE_STRENGTH))
            .force('center', d3.forceCenter(width / 2, height / 2))
            // CHANGED: collision radius handles Conditional nodes.
            .force('collide', d3.forceCollide().radius(function (d) {
                if (d.kind === 'Cell') return CELL_COLLIDE_R;
                if (d.kind === 'Conditional') return COND_COLLIDE_R;
                return REL_COLLIDE_R;
            }));

        simulation.on('tick', ticked);

        update(data);
    }

    function update(data) {
        // Guard: no-op if not yet initialized
        if (!svg) return;

        // Detect structural changes before mutating node/link arrays.
        var oldNodeIds = new Set(nodes.map(function (n) { return n.id; }));
        var oldLinkSet = new Set(links.map(function (l) {
            var src = typeof l.source === 'object' ? l.source.id : l.source;
            var tgt = typeof l.target === 'object' ? l.target.id : l.target;
            return src + '-' + tgt;
        }));
        var structureChanged = nodes.length !== data.nodes.length
            || links.length !== data.links.length
            || data.nodes.some(function (n) { return !oldNodeIds.has(n.id); })
            || data.links.some(function (l) { return !oldLinkSet.has(l.source + '-' + l.target); });

        // Preserve existing node positions by merging into incoming data.
        var oldNodeMap = new Map(nodes.map(function (n) { return [n.id, n]; }));
        nodes = data.nodes.map(function (n) {
            var existing = oldNodeMap.get(n.id);
            if (existing) {
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

        // NEW: Control links (dashed, color-coded by branch)
        controlLinkLayer.selectAll('line')
            .data(controlLinks, function (d) {
                var src = typeof d.source === 'object' ? d.source.id : d.source;
                var tgt = typeof d.target === 'object' ? d.target.id : d.target;
                return src + '-' + tgt;
            })
            .join('line')
            .attr('class', 'link-control')
            .attr('stroke-dasharray', '5 3')
            .attr('stroke', function (d) {
                var idx = (d.branch_index === null || d.branch_index === undefined)
                    ? -1 : d.branch_index % BRANCH_COLORS.length;
                if (d.branch_active) {
                    return idx < 0 ? DEFAULT_BRANCH_COLOR : BRANCH_COLORS[idx];
                }
                return idx < 0 ? DEFAULT_BRANCH_DIM : BRANCH_COLORS_DIM[idx];
            });

        // Join cell rects
        cellLayer.selectAll('rect')
            .data(cellNodes, function (d) { return d.id; })
            .join('rect')
            .attr('class', 'node-cell')
            .attr('width', CELL_W)
            .attr('height', CELL_H)
            .attr('rx', CELL_RX);

        // Join relationship circles
        relLayer.selectAll('circle')
            .data(relNodes, function (d) { return d.id; })
            .join('circle')
            .attr('class', 'node-relationship')
            .attr('r', REL_R);

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

        // Highlight forced cells (see property_model::Sheet::is_forced) and every
        // constraint edge touching one: the incoming edge that produces it, and any
        // outgoing edges carrying its (also guaranteed) value onward to other
        // relationships. Forced cells always belong to a currently active
        // relationship, so this never overlaps with the inactive-relationship
        // dimming above.
        (function () {
            var forcedSet = new Set(data.forced || []);
            cellLayer.selectAll('rect')
                .classed('forced', function (d) { return forcedSet.has(d.id); });
            linkLayer.selectAll('line')
                .classed('forced-edge', function (d) {
                    var srcId = typeof d.source === 'object' ? d.source.id : d.source;
                    var tgtId = typeof d.target === 'object' ? d.target.id : d.target;
                    return forcedSet.has(srcId) || forcedSet.has(tgtId);
                });
        }());

        // NEW: Conditional diamond nodes (rotated rect)
        condLayer.selectAll('rect')
            .data(condNodes, function (d) { return d.id; })
            .join('rect')
            .attr('class', 'node-conditional')
            .attr('width', COND_SIZE * 2)
            .attr('height', COND_SIZE * 2);

        // Join cell name labels (centered inside rect)
        labelLayer.selectAll('text')
            .data(cellNodes, function (d) { return d.id; })
            .join('text')
            .attr('class', 'node-label')
            .text(function (d) { return d.label; });

        // Join cell value labels (below the name, inside rect)
        valueLayer.selectAll('text')
            .data(cellNodes, function (d) { return d.id; })
            .join('text')
            .attr('class', 'node-value')
            .text(function (d) { return d.value || ''; });

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
            // Settle synchronously so the graph is stable before display.
            settleSimulation();
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

        // NEW: Control links: center-to-center (dashed lines, no arrowhead clipping needed).
        controlLinkLayer.selectAll('line').each(function (d) {
            var s = d.source, t = d.target;
            d3.select(this)
                .attr('x1', s.x).attr('y1', s.y)
                .attr('x2', t.x).attr('y2', t.y);
        });

        cellLayer.selectAll('rect')
            .attr('x', function (d) { return d.x - CELL_W / 2; })
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
