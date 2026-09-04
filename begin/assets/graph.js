(function () {
    // Tunable layout constants (shared across every instance).
    var LINK_DISTANCE = 80;
    var CHARGE_STRENGTH = -300;
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
    var PULSE_COLOR = '#f90';
    var PULSE_ON_MS = 200;
    var PULSE_OFF_MS = 400;
    var INACTIVE_STROKE = '#ccc';
    var MAX_ZOOM = 8;

    // One GraphInstance per mounted container id -- begin's single view, or one of a
    // book page's several simultaneously-live examples. Each owns its D3
    // simulation/SVG/layout state entirely independently; nothing here is shared
    // across instances, so switching or dragging one can never affect another.
    var instances = new Map();

    // ---- Pure helpers (no instance state) ----

    function setsEqual(a, b) {
        if (a.size !== b.size) return false;
        for (var v of a) {
            if (!b.has(v)) return false;
        }
        return true;
    }

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

    function cellWidth(d) {
        return d.w || CELL_W;
    }

    function circleEdgePoint(sx, sy, cx, cy, r) {
        var dx = cx - sx, dy = cy - sy;
        var dist = Math.sqrt(dx * dx + dy * dy);
        if (dist < 1) return { x: cx, y: cy };
        return { x: cx - dx / dist * r, y: cy - dy / dist * r };
    }

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

    function dragBehavior(sim) {
        return d3.drag()
            .on('start', function (event) {
                // Both d3.zoom (on the <svg>) and this drag (on the node
                // shape) listen for pointer-down; without this the same
                // gesture would also pan the canvas.
                event.sourceEvent.stopPropagation();
            })
            .on('drag', function (event, d) {
                if (!d3.select(this).classed('dragging')) {
                    sim.alphaTarget(0.3).restart();
                    d3.select(this).classed('dragging', true);
                }
                d.fx = event.x;
                d.fy = event.y;
            })
            .on('end', function (event) {
                if (!event.active) sim.alphaTarget(0);
                d3.select(this).classed('dragging', false);
            });
    }

    // ---- GraphInstance: one independent D3 force layout mounted into one container ----

    function GraphInstance(containerId) {
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
        this.latestData = null;
        // Seeded from a JS global that begin (not graph.js/GraphView -- see their
        // doc comments) sets alongside its "Show inactive" toggle, mirroring the
        // existing window.__beginGraphData seam: this lets a fresh instance (built
        // by init() on a source switch) start from whatever value the toggle is
        // currently showing instead of always resetting to true, without threading
        // a begin-only concern through GraphView's props. Falls back to true (dim,
        // not hide) when the global is unset -- e.g. every book page, which has no
        // such toggle.
        this.showInactive = (typeof window.__beginShowInactive === 'boolean') ? window.__beginShowInactive : true;
        this.hiddenNodeIds = new Set();
    }

    GraphInstance.prototype.computeBBox = function () {
        var self = this;
        var minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
        this.nodes.forEach(function (n) {
            if (self.hiddenNodeIds.has(n.id)) return;
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
        if (!isFinite(minX)) {
            return { minX: 0, minY: 0, maxX: this.width, maxY: this.height };
        }
        return {
            minX: minX - FIT_MARGIN, minY: minY - FIT_MARGIN,
            maxX: maxX + FIT_MARGIN, maxY: maxY + FIT_MARGIN
        };
    };

    GraphInstance.prototype.fitTransformFor = function (bbox) {
        var cx = (bbox.minX + bbox.maxX) / 2;
        var cy = (bbox.minY + bbox.maxY) / 2;
        var contentW = Math.max(bbox.maxX - bbox.minX, 1);
        var contentH = Math.max(bbox.maxY - bbox.minY, 1);
        var fitScale = Math.min(this.width / contentW, this.height / contentH);
        return {
            fitScale: fitScale,
            transform: d3.zoomIdentity.translate(this.width / 2, this.height / 2).scale(fitScale).translate(-cx, -cy)
        };
    };

    GraphInstance.prototype.updateZoomConstraints = function (forceFit) {
        var bbox = this.computeBBox();
        var fit = this.fitTransformFor(bbox);
        var maxScale = Math.max(fit.fitScale, MAX_ZOOM);
        var extent = [[0, 0], [this.width, this.height]];
        var translateExtent = [[bbox.minX, bbox.minY], [bbox.maxX, bbox.maxY]];
        this.zoom.scaleExtent([fit.fitScale, maxScale])
            .translateExtent(translateExtent)
            .extent(extent);
        if (!this.hasInitialFit || forceFit) {
            this.svg.call(this.zoom.transform, fit.transform);
            this.hasInitialFit = true;
        } else {
            var current = d3.zoomTransform(this.svg.node());
            var clampedK = Math.max(fit.fitScale, Math.min(maxScale, current.k));
            var rescaled = current.scale(clampedK / current.k);
            var clamped = this.zoom.constrain()(rescaled, extent, translateExtent);
            this.svg.call(this.zoom.transform, clamped);
        }
    };

    GraphInstance.prototype.settleSimulation = function (forceFit) {
        var n = Math.ceil(Math.log(this.simulation.alphaMin()) / Math.log(1 - this.simulation.alphaDecay()));
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
        // container's size continuously, not just once at mount. The first firing
        // measures after layout has settled -- a plain clientWidth/clientHeight read
        // here can race layout and return a stale (often zero) size -- and builds
        // the graph; every later firing just resizes the existing canvas.
        this.resizeObserver = new ResizeObserver(function () {
            self.width = container.clientWidth || self.width;
            self.height = container.clientHeight || self.height;
            if (!self.svg) {
                self.buildGraph(container, self.latestData);
            } else {
                self.resizeCanvas();
            }
        });
        this.resizeObserver.observe(container);
    };

    // Resizes the existing SVG to the current width/height without touching
    // node positions or restarting the simulation.
    GraphInstance.prototype.resizeCanvas = function () {
        this.svg.attr('width', this.width)
            .attr('height', this.height)
            .attr('viewBox', [0, 0, this.width, this.height]);
        this.simulation.force('center').x(this.width / 2).y(this.height / 2);
        this.updateZoomConstraints();
    };

    GraphInstance.prototype.buildGraph = function (container, data) {
        var self = this;
        this.svg = d3.select(container)
            .append('svg')
            .attr('width', this.width)
            .attr('height', this.height)
            .attr('viewBox', [0, 0, this.width, this.height]);

        var defs = this.svg.append('defs');

        // Arrowhead: refX=10 places the tip (at local x=10) at the line endpoint.
        defs.append('marker')
            .attr('id', 'arrowhead')
            .attr('viewBox', '0 -5 10 10')
            .attr('refX', 10)
            .attr('refY', 0)
            .attr('markerWidth', 8)
            .attr('markerHeight', 8)
            .attr('markerUnits', 'userSpaceOnUse')
            .attr('orient', 'auto')
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
        this.zoomLayer = this.svg.append('g').attr('class', 'zoom-layer');
        this.zoomLayer.append('g').attr('class', 'bg-layer');
        this.controlLinkLayer = this.zoomLayer.append('g').attr('class', 'control-link-layer');
        this.linkLayer = this.zoomLayer.append('g').attr('class', 'link-layer');
        this.cellLayer = this.zoomLayer.append('g').attr('class', 'cell-layer');
        this.relLayer = this.zoomLayer.append('g').attr('class', 'rel-layer');
        this.condLayer = this.zoomLayer.append('g').attr('class', 'cond-layer');
        this.labelLayer = this.zoomLayer.append('g').attr('class', 'label-layer');
        this.valueLayer = this.zoomLayer.append('g').attr('class', 'value-layer');

        this.zoom = d3.zoom().on('zoom', function (event) {
            self.zoomLayer.attr('transform', event.transform);
        });
        this.svg.call(this.zoom);

        this.simulation = d3.forceSimulation()
            .force('link', d3.forceLink().id(function (d) { return d.id; }).distance(function (d) {
                var sKind = typeof d.source === 'object' ? d.source.kind : null;
                var tKind = typeof d.target === 'object' ? d.target.kind : null;
                return (sKind === 'Branch' || tKind === 'Branch') ? LINK_DISTANCE / 2 : LINK_DISTANCE;
            }))
            .force('charge', d3.forceManyBody().strength(function (d) {
                return d.kind === 'Branch' ? 0 : CHARGE_STRENGTH;
            }))
            .force('center', d3.forceCenter(this.width / 2, this.height / 2))
            .force('collide', d3.forceCollide().radius(function (d) {
                if (d.kind === 'Cell') return Math.max(CELL_COLLIDE_R, cellWidth(d) / 2 + 4);
                if (d.kind === 'Conditional') return COND_COLLIDE_R;
                if (d.kind === 'Branch') return 0;
                return REL_COLLIDE_R;
            }));

        this.simulation.on('tick', function () {
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
    GraphInstance.prototype.update = function (data) {
        var self = this;
        this.latestData = data;
        if (!this.svg) return;

        // True only for the very first call after this instance was built (its node
        // list is still empty) -- this instance-local fact replaces the old
        // cross-source "sourceChanged" string comparison, since a fresh instance
        // never carries over another sheet's nodes to begin with.
        var isFirstPopulation = this.nodes.length === 0 && data.nodes.length > 0;

        function linkKey(a, b) { return a < b ? a + '|' + b : b + '|' + a; }
        var oldNodeIds = new Set(this.nodes.map(function (n) { return n.id; }));
        var oldLinkSet = new Set(this.links.map(function (l) {
            var src = typeof l.source === 'object' ? l.source.id : l.source;
            var tgt = typeof l.target === 'object' ? l.target.id : l.target;
            return linkKey(src, tgt);
        }));
        var structureChanged = this.nodes.length !== data.nodes.length
            || this.links.length !== data.links.length
            || data.nodes.some(function (n) { return !oldNodeIds.has(n.id); })
            || data.links.some(function (l) { return !oldLinkSet.has(linkKey(l.source, l.target)); });

        // Node ids are only unique *within* the one Sheet this instance was created
        // for -- they're built from a cell's raw slotmap index (see cell_node_id()
        // in bridge.rs) -- and begin rebuilds a brand-new Sheet from source text on
        // every same-source hot-reload (see App's use_effect in begin/src/app.rs),
        // so an id can be silently recycled for a *different* cell across two
        // consecutive update() calls on this same instance. relabeledIds tracks
        // exactly that case so the width-measuring step below knows to remeasure
        // even though oldNodeMap already has the id.
        var oldNodeMap = new Map(this.nodes.map(function (n) { return [n.id, n]; }));
        var relabeledIds = new Set();
        this.nodes = data.nodes.map(function (n) {
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
        var nodeMap = new Map(this.nodes.map(function (n) { return [n.id, n]; }));
        this.links = data.links.map(function (l) { return Object.assign({}, l); });

        var changedSet = new Set(data.changed || []);

        var controlledIds = new Set();
        var activeIds = new Set();
        this.links.forEach(function (l) {
            if (l.kind !== 'Control') return;
            var tgtId = typeof l.target === 'object' ? l.target.id : l.target;
            controlledIds.add(tgtId);
            if (l.branch_active) activeIds.add(tgtId);
        });
        function isInactive(id) {
            return controlledIds.has(id) && !activeIds.has(id);
        }

        var newHiddenIds = this.showInactive ? new Set() : new Set(
            this.nodes.filter(function (n) { return isInactive(n.id); }).map(function (n) { return n.id; })
        );
        var hiddenSetChanged = !setsEqual(newHiddenIds, this.hiddenNodeIds);
        this.hiddenNodeIds = newHiddenIds;
        structureChanged = structureChanged || hiddenSetChanged;

        function touchesHidden(l) {
            var srcId = typeof l.source === 'object' ? l.source.id : l.source;
            var tgtId = typeof l.target === 'object' ? l.target.id : l.target;
            return self.hiddenNodeIds.has(srcId) || self.hiddenNodeIds.has(tgtId);
        }
        var visibleNodes = this.nodes.filter(function (n) { return !self.hiddenNodeIds.has(n.id); });
        var visibleLinks = this.links.filter(function (l) { return !touchesHidden(l); });

        var cellNodes = visibleNodes.filter(function (n) { return n.kind === 'Cell'; });
        var relNodes = visibleNodes.filter(function (n) { return n.kind === 'Relationship'; });
        var condNodes = visibleNodes.filter(function (n) { return n.kind === 'Conditional'; });
        var constraintLinks = visibleLinks.filter(function (l) { return l.kind === 'Constraint'; });
        var controlLinks = visibleLinks.filter(function (l) { return l.kind === 'Control'; });

        this.linkLayer.selectAll('line')
            .data(constraintLinks, function (d) {
                var src = typeof d.source === 'object' ? d.source.id : d.source;
                var tgt = typeof d.target === 'object' ? d.target.id : d.target;
                return src + '-' + tgt;
            })
            .join('line')
            .attr('class', 'link');

        this.controlLinkLayer.selectAll('line')
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

        var labelSel = this.labelLayer.selectAll('text')
            .data(cellNodes, function (d) { return d.id; })
            .join('text')
            .attr('class', 'node-label')
            .text(function (d) { return d.label; });

        var valueSel = this.valueLayer.selectAll('text')
            .data(cellNodes, function (d) { return d.id; })
            .join('text')
            .attr('class', 'node-value')
            .text(function (d) { return d.value || ''; });

        labelSel.each(function (d) {
            if (oldNodeMap.has(d.id) && !relabeledIds.has(d.id)) return;
            d.w = Math.max(CELL_W, this.getBBox().width + CELL_LABEL_PADDING);
        });
        valueSel.each(function (d) {
            if (oldNodeMap.has(d.id) && !changedSet.has(d.id) && !relabeledIds.has(d.id)) return;
            d.w = Math.max(d.w, this.getBBox().width + CELL_LABEL_PADDING);
        });

        this.cellLayer.selectAll('rect')
            .data(cellNodes, function (d) { return d.id; })
            .join('rect')
            .attr('class', 'node-cell')
            .attr('width', cellWidth)
            .attr('height', CELL_H)
            .attr('rx', CELL_RX)
            .call(dragBehavior(this.simulation))
            .on('dblclick', function (event, d) { self.unpinNode(event, d); });

        this.relLayer.selectAll('circle')
            .data(relNodes, function (d) { return d.id; })
            .join('circle')
            .attr('class', 'node-relationship')
            .attr('r', REL_R)
            .call(dragBehavior(this.simulation))
            .on('dblclick', function (event, d) { self.unpinNode(event, d); });

        (function () {
            self.relLayer.selectAll('circle').style('stroke', function (d) {
                return isInactive(d.id) ? INACTIVE_STROKE : null;
            });
            self.linkLayer.selectAll('line')
                .style('stroke', function (d) {
                    var srcId = typeof d.source === 'object' ? d.source.id : d.source;
                    var tgtId = typeof d.target === 'object' ? d.target.id : d.target;
                    return (isInactive(srcId) || isInactive(tgtId)) ? INACTIVE_STROKE : null;
                })
                .attr('marker-end', function (d) {
                    if (!data.arrows) return null;
                    var srcId = typeof d.source === 'object' ? d.source.id : d.source;
                    var tgtId = typeof d.target === 'object' ? d.target.id : d.target;
                    if (isInactive(srcId) || isInactive(tgtId)) return null;
                    var tgtNode = nodeMap.get(tgtId);
                    return tgtNode ? 'url(#arrowhead)' : null;
                });
        }());

        (function () {
            var forcedSet = new Set(data.forced || []);
            var forcedRelSet = new Set(data.forced_relationships || []);
            self.cellLayer.selectAll('rect')
                .classed('forced', function (d) { return forcedSet.has(d.id); });
            self.relLayer.selectAll('circle')
                .classed('forced', function (d) { return forcedRelSet.has(d.id); });
            self.linkLayer.selectAll('line')
                .classed('forced-edge', function (d) {
                    var srcId = typeof d.source === 'object' ? d.source.id : d.source;
                    var tgtId = typeof d.target === 'object' ? d.target.id : d.target;
                    return forcedSet.has(srcId) || forcedSet.has(tgtId)
                        || forcedRelSet.has(srcId) || forcedRelSet.has(tgtId);
                });
        }());

        this.condLayer.selectAll('rect')
            .data(condNodes, function (d) { return d.id; })
            .join('rect')
            .attr('class', 'node-conditional')
            .attr('width', COND_SIZE * 2)
            .attr('height', COND_SIZE * 2)
            .call(dragBehavior(this.simulation))
            .on('dblclick', function (event, d) { self.unpinNode(event, d); });

        if (changedSet.size > 0) {
            this.cellLayer.selectAll('rect')
                .filter(function (d) { return changedSet.has(d.id); })
                .transition().duration(PULSE_ON_MS)
                .style('fill', PULSE_COLOR)
                .transition().duration(PULSE_OFF_MS)
                .style('fill', null);
        }

        this.simulation.nodes(visibleNodes);
        this.simulation.force('link').links(visibleLinks);

        if (isFirstPopulation) {
            // Nothing to animate from -- settle synchronously and snap the view to fit.
            this.settleSimulation(true);
        } else if (structureChanged) {
            this.ticked();
            this.simulation.alpha(1).restart();
        } else {
            this.ticked();
        }
    };

    GraphInstance.prototype.ticked = function () {
        this.linkLayer.selectAll('line').each(function (d) {
            var ep = linkEndpoints(d);
            d3.select(this)
                .attr('x1', ep.x1).attr('y1', ep.y1)
                .attr('x2', ep.x2).attr('y2', ep.y2);
        });

        this.controlLinkLayer.selectAll('line').each(function (d) {
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

        this.cellLayer.selectAll('rect')
            .attr('x', function (d) { return d.x - cellWidth(d) / 2; })
            .attr('y', function (d) { return d.y - CELL_H / 2; });

        this.relLayer.selectAll('circle')
            .attr('cx', function (d) { return d.x; })
            .attr('cy', function (d) { return d.y; });

        this.condLayer.selectAll('rect')
            .attr('transform', function (d) {
                return 'translate(' + d.x + ',' + d.y + ') rotate(45) translate(' + (-COND_SIZE) + ',' + (-COND_SIZE) + ')';
            });

        this.labelLayer.selectAll('text')
            .attr('x', function (d) { return d.x; })
            .attr('y', function (d) { return d.y - 4; });

        this.valueLayer.selectAll('text')
            .attr('x', function (d) { return d.x; })
            .attr('y', function (d) { return d.y + 10; });
    };

    GraphInstance.prototype.zoomIn = function () {
        if (!this.svg || !this.zoom) return;
        this.svg.transition().duration(200).call(this.zoom.scaleBy, 1.3);
    };

    GraphInstance.prototype.zoomOut = function () {
        if (!this.svg || !this.zoom) return;
        this.svg.transition().duration(200).call(this.zoom.scaleBy, 1 / 1.3);
    };

    GraphInstance.prototype.resetZoom = function () {
        if (!this.svg || !this.zoom) return;
        var fit = this.fitTransformFor(this.computeBBox());
        this.svg.transition().duration(300).call(this.zoom.transform, fit.transform);
    };

    GraphInstance.prototype.setShowInactive = function (value) {
        this.showInactive = value;
        if (this.svg) this.update(this.latestData);
    };

    GraphInstance.prototype.destroy = function () {
        if (this.resizeObserver) { this.resizeObserver.disconnect(); this.resizeObserver = null; }
        if (this.simulation) { this.simulation.stop(); this.simulation = null; }
        if (this.svg) { this.svg.remove(); this.svg = null; }
    };

    // ---- Public registry: window.beginGraph, keyed by container id ----

    function init(id, data) {
        var existing = instances.get(id);
        if (existing) existing.destroy();
        var inst = new GraphInstance(id);
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

    function zoomIn(id) { var inst = instances.get(id); if (inst) inst.zoomIn(); }
    function zoomOut(id) { var inst = instances.get(id); if (inst) inst.zoomOut(); }
    function resetZoom(id) { var inst = instances.get(id); if (inst) inst.resetZoom(); }
    function setShowInactive(id, value) { var inst = instances.get(id); if (inst) inst.setShowInactive(value); }

    window.beginGraph = {
        init: init, update: update, destroy: destroy,
        zoomIn: zoomIn, zoomOut: zoomOut, resetZoom: resetZoom,
        setShowInactive: setShowInactive,
    };
}());
