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

        // Arrowhead marker reserved for method-direction arrows
        var defs = svg.append('defs');
        defs.append('marker')
            .attr('id', 'arrowhead')
            .attr('viewBox', '0 -5 10 10')
            .attr('refX', 20).attr('refY', 0)
            .attr('markerWidth', 6).attr('markerHeight', 6)
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

        // Preserve existing node positions by merging into incoming data
        var nodeMap = new Map(nodes.map(function (n) { return [n.id, n]; }));
        nodes = data.nodes.map(function (n) {
            var existing = nodeMap.get(n.id);
            if (existing) {
                existing.kind = n.kind;
                existing.label = n.label;
                existing.value = n.value;
                return existing;
            }
            return Object.assign({}, n);
        });
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
            .attr('class', 'link');

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

        // Restart simulation with updated data
        simulation.nodes(nodes);
        simulation.force('link').links(links);
        simulation.alpha(0.3).restart();
    }

    function ticked() {
        linkLayer.selectAll('line')
            .attr('x1', function (d) { return d.source.x; })
            .attr('y1', function (d) { return d.source.y; })
            .attr('x2', function (d) { return d.target.x; })
            .attr('y2', function (d) { return d.target.y; });

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
