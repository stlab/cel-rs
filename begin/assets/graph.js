(function () {
    // Tunable layout constants
    var LINK_DISTANCE = 80;
    var CHARGE_STRENGTH = -300;
    var CELL_W = 60;
    var CELL_H = 36;
    var CELL_RX = 4;
    var REL_R = 16;
    var CELL_COLLIDE_R = 38;
    var REL_COLLIDE_R = 22;
    var PULSE_COLOR = '#f90';
    var PULSE_ON_MS = 200;
    var PULSE_OFF_MS = 400;

    var svg = null;
    var simulation = null;
    var linkLayer = null;
    var cellLayer = null;
    var relLayer = null;
    var labelLayer = null;
    var valueLayer = null;
    var nodes = [];
    var links = [];
    var width = 800;
    var height = 600;

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

    // Returns edge-to-edge endpoints for a link after D3 resolves source/target to objects.
    function linkEndpoints(d) {
        var s = d.source, t = d.target;
        var srcPt = s.kind === 'Cell'
            ? cellEdgePoint(t.x, t.y, s.x, s.y)
            : circleEdgePoint(t.x, t.y, s.x, s.y, REL_R);
        var tgtPt = t.kind === 'Cell'
            ? cellEdgePoint(s.x, s.y, t.x, t.y)
            : circleEdgePoint(s.x, s.y, t.x, t.y, REL_R);
        return { x1: srcPt.x, y1: srcPt.y, x2: tgtPt.x, y2: tgtPt.y };
    }

    // Runs the simulation synchronously until settled, then updates the display.
    function settleSimulation() {
        var n = Math.ceil(Math.log(simulation.alphaMin()) / Math.log(1 - simulation.alphaDecay()));
        simulation.stop().alpha(1).tick(n);
        ticked();
    }

    function init(containerId, data) {
        // Tear down any previous init (component remount / hot-reload).
        if (simulation) { simulation.stop(); simulation = null; }
        if (svg) { svg.remove(); svg = null; }
        nodes = [];
        links = [];

        var container = document.getElementById(containerId);
        width = container.clientWidth || width;
        height = container.clientHeight || height;

        svg = d3.select(container)
            .append('svg')
            .attr('width', '100%')
            .attr('height', '100%')
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

        // Layer groups in z-order: background → links → cells → relationships → labels → values
        svg.append('g').attr('class', 'bg-layer');
        linkLayer = svg.append('g').attr('class', 'link-layer');
        cellLayer = svg.append('g').attr('class', 'cell-layer');
        relLayer = svg.append('g').attr('class', 'rel-layer');
        labelLayer = svg.append('g').attr('class', 'label-layer');
        valueLayer = svg.append('g').attr('class', 'value-layer');

        simulation = d3.forceSimulation()
            .force('link', d3.forceLink().id(function (d) { return d.id; }).distance(LINK_DISTANCE))
            .force('charge', d3.forceManyBody().strength(CHARGE_STRENGTH))
            .force('center', d3.forceCenter(width / 2, height / 2))
            .force('collide', d3.forceCollide()
                .radius(function (d) { return d.kind === 'Cell' ? CELL_COLLIDE_R : REL_COLLIDE_R; }));

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

        // Join links
        linkLayer.selectAll('line')
            .data(links, function (d) {
                var src = typeof d.source === 'object' ? d.source.id : d.source;
                var tgt = typeof d.target === 'object' ? d.target.id : d.target;
                return src + '-' + tgt;
            })
            .join('line')
            .attr('class', 'link')
            .attr('marker-end', function (d) {
                if (!data.arrows) return null;
                var tgtId = typeof d.target === 'object' ? d.target.id : d.target;
                var tgtNode = nodeMap.get(tgtId);
                return tgtNode ? 'url(#arrowhead)' : null;
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

        // Always update simulation data so that link source/target are resolved
        // from string IDs to node objects before ticked() accesses d.source.x etc.
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
        // Draw lines edge-to-edge so arrowhead tips land exactly at node boundaries.
        linkLayer.selectAll('line').each(function (d) {
            var ep = linkEndpoints(d);
            d3.select(this)
                .attr('x1', ep.x1).attr('y1', ep.y1)
                .attr('x2', ep.x2).attr('y2', ep.y2);
        });

        cellLayer.selectAll('rect')
            .attr('x', function (d) { return d.x - CELL_W / 2; })
            .attr('y', function (d) { return d.y - CELL_H / 2; });

        relLayer.selectAll('circle')
            .attr('cx', function (d) { return d.x; })
            .attr('cy', function (d) { return d.y; });

        // Cell name: upper half of rect
        labelLayer.selectAll('text')
            .attr('x', function (d) { return d.x; })
            .attr('y', function (d) { return d.y - 4; });

        // Cell value: lower half of rect
        valueLayer.selectAll('text')
            .attr('x', function (d) { return d.x; })
            .attr('y', function (d) { return d.y + 10; });
    }

    window.beginGraph = { init: init, update: update };
}());
