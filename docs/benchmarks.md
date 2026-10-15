# Benchmarks

The corpus is every plain fixture of the test suite: process models discovered from public event
logs and synthetic graphs, from a few nodes to two thousand. graphlay,
[dagre](https://github.com/dagrejs/dagre) and [ELK](https://eclipse.dev/elk/) (layered, orthogonal
routing) each lay out every case with the same node sizes and the same node and layer gaps.
graphlay uses the options the fixture names, which for a directly-follows graph is the process-map
preset. graphlay's own metrics measure all three results.

Lower is better everywhere. Time is the median of several runs in Node, graphlay as WebAssembly,
the others as JavaScript. Area is the bounding box of nodes and edges. "graphlay fast" is the same
engine with `effort: "fast"`, the setting for an interactive relayout.

dagre's edge points are control points for a smooth curve. Counted as a polyline they bend less
and cut through more nodes than the drawn curve would. On a directly-follows graph, graphlay uses
edge weights to keep the main path straight, which costs it bends and saves crossings. dagre and
ELK do not read weights that way.

Versions: graphlay 0.2.0, @dagrejs/dagre 3.1.1, elkjs 0.12.0.
Regenerate with `pnpm --filter bench bench`.

## Over the whole corpus

How many cases each engine wins, with a tie counting for every engine that shares it, and the
totals.

| engine | time (ms) wins | crossings wins | bends wins | edges through nodes wins | edge length (px) wins | area (kpx²) wins |
|---|---:|---:|---:|---:|---:|---:|
| graphlay | 7 of 55 | 49 of 55 | 19 of 55 | 53 of 55 | 17 of 55 | 16 of 55 |
| graphlay fast | 46 of 55 | 33 of 55 | 18 of 55 | 52 of 55 | 15 of 55 | 15 of 55 |
| dagre | 0 of 55 | 22 of 55 | 37 of 55 | 30 of 55 | 21 of 55 | 12 of 55 |
| elk | 0 of 55 | 28 of 55 | 6 of 55 | 53 of 55 | 13 of 55 | 27 of 55 |

| engine | total time (ms) | total crossings | total bends | total edges through nodes | total edge length (px) | total area (kpx²) |
|---|---:|---:|---:|---:|---:|---:|
| graphlay | 2857 | 9301 | 7523 | 0 | 2,494,153 | 265,669 |
| graphlay fast | 916 | 9692 | 7686 | 4 | 2,524,043 | 299,693 |
| dagre | 10831 | 12689 | 5514 | 2296 | 3,647,109 | 503,209 |
| elk | 4182 | 12126 | 8476 | 0 | 3,918,776 | 519,560 |

## Case by case

| case | nodes | edges | engine | time (ms) | crossings | bends | edges through nodes | edge length (px) | area (kpx²) |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|
| bundled-lane-off-outline | 3 | 7 | graphlay | 1.0 | 0 | 2 | 0 | 1,315 | 55 |
| bundled-lane-off-outline | 3 | 7 | graphlay fast | 0.8 | 0 | 2 | 0 | 1,315 | 55 |
| bundled-lane-off-outline | 3 | 7 | dagre | 3.1 | 0 | 8 | 0 | 1,159 | 55 |
| bundled-lane-off-outline | 3 | 7 | elk | 13 | 0 | 20 | 0 | 1,468 | 49 |
| edge-label-gap-reservation | 3 | 3 | graphlay | 0.7 | 0 | 4 | 0 | 464 | 41 |
| edge-label-gap-reservation | 3 | 3 | graphlay fast | 0.7 | 0 | 4 | 0 | 464 | 41 |
| edge-label-gap-reservation | 3 | 3 | dagre | 2.0 | 0 | 4 | 0 | 400 | 42 |
| edge-label-gap-reservation | 3 | 3 | elk | 9.2 | 0 | 4 | 0 | 433 | 30 |
| dfg-small-example-all | 5 | 6 | graphlay | 0.8 | 0 | 6 | 0 | 828 | 123 |
| dfg-small-example-all | 5 | 6 | graphlay fast | 0.7 | 0 | 6 | 0 | 828 | 123 |
| dfg-small-example-all | 5 | 6 | dagre | 2.6 | 0 | 5 | 0 | 770 | 151 |
| dfg-small-example-all | 5 | 6 | elk | 11 | 0 | 6 | 0 | 804 | 119 |
| dfg-small-example-all-diag | 5 | 6 | graphlay | 0.7 | 0 | 3 | 0 | 659 | 123 |
| dfg-small-example-all-diag | 5 | 6 | graphlay fast | 0.6 | 0 | 3 | 0 | 659 | 123 |
| dfg-small-example-all-diag | 5 | 6 | dagre | 2.3 | 0 | 5 | 0 | 770 | 151 |
| dfg-small-example-all-diag | 5 | 6 | elk | 11 | 0 | 6 | 0 | 804 | 119 |
| dfg-small-example-min5pct | 5 | 6 | graphlay | 0.7 | 0 | 6 | 0 | 828 | 123 |
| dfg-small-example-min5pct | 5 | 6 | graphlay fast | 0.6 | 0 | 6 | 0 | 828 | 123 |
| dfg-small-example-min5pct | 5 | 6 | dagre | 2.7 | 0 | 5 | 0 | 770 | 151 |
| dfg-small-example-min5pct | 5 | 6 | elk | 11 | 0 | 6 | 0 | 804 | 119 |
| dfg-small-example-top25pct | 5 | 5 | graphlay | 0.4 | 0 | 6 | 0 | 644 | 128 |
| dfg-small-example-top25pct | 5 | 5 | graphlay fast | 0.4 | 0 | 6 | 0 | 644 | 128 |
| dfg-small-example-top25pct | 5 | 5 | dagre | 1.5 | 0 | 5 | 0 | 763 | 152 |
| dfg-small-example-top25pct | 5 | 5 | elk | 8.2 | 0 | 6 | 0 | 818 | 131 |
| dfg-an1-all | 8 | 18 | graphlay | 1.9 | 0 | 30 | 0 | 4,878 | 349 |
| dfg-an1-all | 8 | 18 | graphlay fast | 1.9 | 0 | 30 | 0 | 4,878 | 349 |
| dfg-an1-all | 8 | 18 | dagre | 9.8 | 5 | 24 | 0 | 3,718 | 318 |
| dfg-an1-all | 8 | 18 | elk | 22 | 3 | 34 | 0 | 3,538 | 216 |
| dfg-an1-all-diag | 8 | 18 | graphlay | 1.5 | 0 | 15 | 0 | 4,032 | 349 |
| dfg-an1-all-diag | 8 | 18 | graphlay fast | 1.4 | 0 | 15 | 0 | 4,032 | 349 |
| dfg-an1-all-diag | 8 | 18 | dagre | 8.9 | 5 | 24 | 0 | 3,718 | 318 |
| dfg-an1-all-diag | 8 | 18 | elk | 18 | 3 | 34 | 0 | 3,538 | 216 |
| dfg-an1-min5pct | 8 | 18 | graphlay | 1.4 | 0 | 30 | 0 | 4,878 | 349 |
| dfg-an1-min5pct | 8 | 18 | graphlay fast | 1.4 | 0 | 30 | 0 | 4,878 | 349 |
| dfg-an1-min5pct | 8 | 18 | dagre | 9.1 | 5 | 24 | 0 | 3,718 | 318 |
| dfg-an1-min5pct | 8 | 18 | elk | 30 | 3 | 34 | 0 | 3,538 | 216 |
| dfg-an1-top25pct | 8 | 6 | graphlay | 0.6 | 0 | 2 | 0 | 520 | 212 |
| dfg-an1-top25pct | 8 | 6 | graphlay fast | 0.6 | 0 | 2 | 0 | 520 | 212 |
| dfg-an1-top25pct | 8 | 6 | dagre | 2.7 | 0 | 2 | 0 | 443 | 222 |
| dfg-an1-top25pct | 8 | 6 | elk | 8.3 | 0 | 2 | 0 | 518 | 260 |
| parallel-selfloops | 8 | 29 | graphlay | 1.1 | 0 | 16 | 0 | 1,662 | 107 |
| parallel-selfloops | 8 | 29 | graphlay fast | 1.2 | 0 | 16 | 0 | 1,662 | 107 |
| parallel-selfloops | 8 | 29 | dagre | 10 | 0 | 22 | 0 | 2,717 | 169 |
| parallel-selfloops | 8 | 29 | elk | 14 | 0 | 16 | 0 | 1,433 | 103 |
| multihop-run-through-rank | 9 | 8 | graphlay | 0.6 | 0 | 6 | 0 | 975 | 201 |
| multihop-run-through-rank | 9 | 8 | graphlay fast | 0.5 | 0 | 6 | 0 | 975 | 201 |
| multihop-run-through-rank | 9 | 8 | dagre | 3.3 | 1 | 6 | 0 | 832 | 198 |
| multihop-run-through-rank | 9 | 8 | elk | 10 | 0 | 8 | 0 | 1,162 | 234 |
| chain-10 | 10 | 9 | graphlay | 0.4 | 0 | 0 | 0 | 486 | 121 |
| chain-10 | 10 | 9 | graphlay fast | 0.3 | 0 | 0 | 0 | 486 | 121 |
| chain-10 | 10 | 9 | dagre | 2.7 | 0 | 0 | 0 | 486 | 121 |
| chain-10 | 10 | 9 | elk | 8.4 | 0 | 0 | 0 | 486 | 121 |
| chain-10-lr | 10 | 9 | graphlay | 0.4 | 0 | 0 | 0 | 486 | 88 |
| chain-10-lr | 10 | 9 | graphlay fast | 0.4 | 0 | 0 | 0 | 486 | 88 |
| chain-10-lr | 10 | 9 | dagre | 2.6 | 0 | 0 | 0 | 486 | 88 |
| chain-10-lr | 10 | 9 | elk | 7.3 | 0 | 0 | 0 | 486 | 88 |
| dfg-repairexample-all | 10 | 28 | graphlay | 4.1 | 4 | 51 | 0 | 7,210 | 553 |
| dfg-repairexample-all | 10 | 28 | graphlay fast | 2.6 | 5 | 54 | 0 | 8,254 | 559 |
| dfg-repairexample-all | 10 | 28 | dagre | 19 | 20 | 40 | 0 | 7,824 | 548 |
| dfg-repairexample-all | 10 | 28 | elk | 28 | 6 | 54 | 0 | 5,034 | 342 |
| dfg-repairexample-all-diag | 10 | 28 | graphlay | 6.3 | 4 | 29 | 0 | 6,163 | 553 |
| dfg-repairexample-all-diag | 10 | 28 | graphlay fast | 2.9 | 7 | 27 | 0 | 6,755 | 559 |
| dfg-repairexample-all-diag | 10 | 28 | dagre | 23 | 20 | 40 | 0 | 7,824 | 548 |
| dfg-repairexample-all-diag | 10 | 28 | elk | 27 | 6 | 54 | 0 | 5,034 | 342 |
| dfg-repairexample-min5pct | 10 | 26 | graphlay | 2.4 | 4 | 40 | 0 | 6,042 | 504 |
| dfg-repairexample-min5pct | 10 | 26 | graphlay fast | 1.3 | 4 | 40 | 0 | 6,042 | 504 |
| dfg-repairexample-min5pct | 10 | 26 | dagre | 7.1 | 18 | 32 | 0 | 7,087 | 538 |
| dfg-repairexample-min5pct | 10 | 26 | elk | 16 | 4 | 44 | 0 | 4,934 | 359 |
| dfg-repairexample-top25pct | 10 | 11 | graphlay | 0.6 | 0 | 12 | 0 | 1,483 | 302 |
| dfg-repairexample-top25pct | 10 | 11 | graphlay fast | 0.4 | 0 | 12 | 0 | 1,483 | 302 |
| dfg-repairexample-top25pct | 10 | 11 | dagre | 3.2 | 3 | 10 | 1 | 1,979 | 348 |
| dfg-repairexample-top25pct | 10 | 11 | elk | 11 | 0 | 14 | 0 | 1,688 | 279 |
| pn-small-example-im | 11 | 12 | graphlay | 0.8 | 0 | 7 | 0 | 1,769 | 195 |
| pn-small-example-im | 11 | 12 | graphlay fast | 0.5 | 0 | 7 | 0 | 1,769 | 195 |
| pn-small-example-im | 11 | 12 | dagre | 4.4 | 0 | 9 | 0 | 1,548 | 195 |
| pn-small-example-im | 11 | 12 | elk | 11 | 0 | 12 | 0 | 2,117 | 196 |
| bk-infeasible-layer | 12 | 11 | graphlay | 1.1 | 0 | 12 | 0 | 1,886 | 215 |
| bk-infeasible-layer | 12 | 11 | graphlay fast | 1.3 | 0 | 12 | 0 | 1,886 | 215 |
| bk-infeasible-layer | 12 | 11 | dagre | 6.6 | 0 | 5 | 0 | 1,154 | 257 |
| bk-infeasible-layer | 12 | 11 | elk | 11 | 0 | 12 | 0 | 1,894 | 296 |
| pn-repairexample-alphappp | 12 | 5 | graphlay | 0.4 | 0 | 2 | 0 | 486 | 289 |
| pn-repairexample-alphappp | 12 | 5 | graphlay fast | 0.3 | 0 | 2 | 0 | 486 | 289 |
| pn-repairexample-alphappp | 12 | 5 | dagre | 3.1 | 0 | 2 | 0 | 446 | 395 |
| pn-repairexample-alphappp | 12 | 5 | elk | 9.7 | 0 | 2 | 0 | 351 | 159 |
| dfg-rtfm-all | 13 | 78 | graphlay | 113 | 120 | 159 | 0 | 58,980 | 2,166 |
| dfg-rtfm-all | 13 | 78 | graphlay fast | 36 | 132 | 208 | 0 | 69,413 | 2,593 |
| dfg-rtfm-all | 13 | 78 | dagre | 74 | 272 | 123 | 2 | 54,564 | 1,804 |
| dfg-rtfm-all | 13 | 78 | elk | 113 | 226 | 226 | 0 | 54,668 | 1,389 |
| dfg-rtfm-all-diag | 13 | 78 | graphlay | 72 | 129 | 75 | 0 | 51,626 | 2,220 |
| dfg-rtfm-all-diag | 13 | 78 | graphlay fast | 33 | 140 | 86 | 0 | 58,125 | 2,593 |
| dfg-rtfm-all-diag | 13 | 78 | dagre | 61 | 272 | 123 | 2 | 54,564 | 1,804 |
| dfg-rtfm-all-diag | 13 | 78 | elk | 81 | 226 | 226 | 0 | 54,668 | 1,389 |
| dfg-rtfm-min5pct | 13 | 14 | graphlay | 0.9 | 0 | 16 | 0 | 5,571 | 1,269 |
| dfg-rtfm-min5pct | 13 | 14 | graphlay fast | 0.7 | 0 | 16 | 0 | 5,571 | 1,269 |
| dfg-rtfm-min5pct | 13 | 14 | dagre | 5.2 | 0 | 10 | 0 | 5,016 | 1,113 |
| dfg-rtfm-min5pct | 13 | 14 | elk | 9.7 | 0 | 18 | 0 | 6,670 | 1,198 |
| dfg-rtfm-top25pct | 13 | 26 | graphlay | 3.1 | 0 | 30 | 0 | 6,896 | 836 |
| dfg-rtfm-top25pct | 13 | 26 | graphlay fast | 2.2 | 1 | 28 | 0 | 6,426 | 892 |
| dfg-rtfm-top25pct | 13 | 26 | dagre | 12 | 4 | 35 | 0 | 7,684 | 1,009 |
| dfg-rtfm-top25pct | 13 | 26 | elk | 20 | 1 | 46 | 0 | 7,193 | 864 |
| pn-bpic12-tsinghua | 16 | 28 | graphlay | 4.8 | 6 | 39 | 0 | 7,185 | 605 |
| pn-bpic12-tsinghua | 16 | 28 | graphlay fast | 2.4 | 6 | 39 | 0 | 7,185 | 605 |
| pn-bpic12-tsinghua | 16 | 28 | dagre | 11 | 11 | 33 | 2 | 7,454 | 668 |
| pn-bpic12-tsinghua | 16 | 28 | elk | 14 | 17 | 52 | 0 | 6,705 | 556 |
| diamond-blocks | 17 | 24 | graphlay | 0.7 | 0 | 32 | 0 | 4,560 | 407 |
| diamond-blocks | 17 | 24 | graphlay fast | 0.7 | 0 | 32 | 0 | 4,560 | 407 |
| diamond-blocks | 17 | 24 | dagre | 9.1 | 0 | 16 | 0 | 2,644 | 407 |
| diamond-blocks | 17 | 24 | elk | 12 | 0 | 32 | 0 | 4,016 | 774 |
| typegraph-ocel2-p2p | 17 | 34 | graphlay | 14 | 15 | 16 | 0 | 13,971 | 1,277 |
| typegraph-ocel2-p2p | 17 | 34 | graphlay fast | 3.2 | 17 | 18 | 0 | 13,966 | 1,307 |
| typegraph-ocel2-p2p | 17 | 34 | dagre | 15 | 55 | 48 | 1 | 19,452 | 1,282 |
| typegraph-ocel2-p2p | 17 | 34 | elk | 28 | 15 | 60 | 0 | 16,076 | 1,397 |
| typegraph-order-management | 17 | 46 | graphlay | 18 | 75 | 40 | 0 | 29,250 | 1,445 |
| typegraph-order-management | 17 | 46 | graphlay fast | 3.4 | 112 | 41 | 4 | 29,340 | 1,464 |
| typegraph-order-management | 17 | 46 | dagre | 17 | 142 | 67 | 4 | 27,897 | 1,412 |
| typegraph-order-management | 17 | 46 | elk | 28 | 88 | 116 | 0 | 39,829 | 1,322 |
| dfg-sepsis-all | 18 | 135 | graphlay | 319 | 532 | 369 | 0 | 156,896 | 4,962 |
| dfg-sepsis-all | 18 | 135 | graphlay fast | 54 | 570 | 379 | 0 | 160,279 | 5,232 |
| dfg-sepsis-all | 18 | 135 | dagre | 137 | 809 | 227 | 2 | 132,040 | 3,247 |
| dfg-sepsis-all | 18 | 135 | elk | 204 | 652 | 390 | 0 | 146,701 | 3,618 |
| dfg-sepsis-all-diag | 18 | 135 | graphlay | 411 | 584 | 158 | 0 | 136,403 | 4,962 |
| dfg-sepsis-all-diag | 18 | 135 | graphlay fast | 63 | 653 | 157 | 0 | 138,731 | 5,232 |
| dfg-sepsis-all-diag | 18 | 135 | dagre | 119 | 809 | 227 | 2 | 132,040 | 3,247 |
| dfg-sepsis-all-diag | 18 | 135 | elk | 326 | 652 | 390 | 0 | 146,701 | 3,618 |
| dfg-sepsis-min5pct | 18 | 50 | graphlay | 33 | 20 | 98 | 0 | 25,637 | 2,340 |
| dfg-sepsis-min5pct | 18 | 50 | graphlay fast | 11 | 20 | 96 | 0 | 26,457 | 2,184 |
| dfg-sepsis-min5pct | 18 | 50 | dagre | 34 | 69 | 73 | 0 | 26,319 | 1,939 |
| dfg-sepsis-min5pct | 18 | 50 | elk | 37 | 21 | 110 | 0 | 23,859 | 2,079 |
| dfg-sepsis-top25pct | 18 | 49 | graphlay | 25 | 19 | 94 | 0 | 24,939 | 2,281 |
| dfg-sepsis-top25pct | 18 | 49 | graphlay fast | 8.3 | 19 | 94 | 0 | 25,283 | 2,162 |
| dfg-sepsis-top25pct | 18 | 49 | dagre | 30 | 62 | 72 | 0 | 25,392 | 1,932 |
| dfg-sepsis-top25pct | 18 | 49 | elk | 42 | 27 | 114 | 0 | 28,503 | 1,929 |
| dag-20 | 20 | 30 | graphlay | 5.9 | 5 | 44 | 0 | 5,700 | 722 |
| dag-20 | 20 | 30 | graphlay fast | 2.0 | 5 | 44 | 0 | 5,700 | 722 |
| dag-20 | 20 | 30 | dagre | 13 | 7 | 33 | 2 | 7,307 | 835 |
| dag-20 | 20 | 30 | elk | 21 | 5 | 40 | 0 | 6,247 | 600 |
| typegraph-container-logistics | 21 | 34 | graphlay | 9.2 | 28 | 15 | 0 | 15,732 | 1,454 |
| typegraph-container-logistics | 21 | 34 | graphlay fast | 3.7 | 30 | 17 | 0 | 13,832 | 1,412 |
| typegraph-container-logistics | 21 | 34 | dagre | 13 | 73 | 41 | 1 | 17,940 | 1,464 |
| typegraph-container-logistics | 21 | 34 | elk | 30 | 23 | 66 | 0 | 17,659 | 1,299 |
| wide-shallow-30 | 31 | 30 | graphlay | 1.4 | 0 | 56 | 0 | 45,740 | 1,697 |
| wide-shallow-30 | 31 | 30 | graphlay fast | 1.5 | 0 | 56 | 0 | 45,740 | 1,697 |
| wide-shallow-30 | 31 | 30 | dagre | 8.5 | 0 | 30 | 0 | 36,454 | 780 |
| wide-shallow-30 | 31 | 30 | elk | 17 | 0 | 58 | 0 | 79,526 | 1,993 |
| grid-6x6 | 36 | 180 | graphlay | 30 | 1095 | 380 | 0 | 91,824 | 1,474 |
| grid-6x6 | 36 | 180 | graphlay fast | 11 | 1095 | 380 | 0 | 91,824 | 1,474 |
| grid-6x6 | 36 | 180 | dagre | 60 | 923 | 152 | 162 | 197,807 | 2,514 |
| grid-6x6 | 36 | 180 | elk | 92 | 1135 | 380 | 0 | 123,554 | 4,492 |
| hub-spoke-40 | 41 | 40 | graphlay | 3.7 | 0 | 38 | 0 | 33,953 | 1,437 |
| hub-spoke-40 | 41 | 40 | graphlay fast | 3.6 | 0 | 38 | 0 | 33,953 | 1,437 |
| hub-spoke-40 | 41 | 40 | dagre | 21 | 0 | 40 | 0 | 30,521 | 1,378 |
| hub-spoke-40 | 41 | 40 | elk | 24 | 0 | 76 | 0 | 48,290 | 2,185 |
| pn-rtfm-imf02-reference | 42 | 52 | graphlay | 3.1 | 0 | 31 | 0 | 9,105 | 1,457 |
| pn-rtfm-imf02-reference | 42 | 52 | graphlay fast | 3.1 | 0 | 31 | 0 | 9,105 | 1,457 |
| pn-rtfm-imf02-reference | 42 | 52 | dagre | 26 | 0 | 39 | 0 | 8,290 | 1,614 |
| pn-rtfm-imf02-reference | 42 | 52 | elk | 20 | 0 | 44 | 0 | 9,481 | 1,229 |
| pn-sepsis-discovered | 45 | 71 | graphlay | 18 | 37 | 84 | 0 | 20,646 | 2,761 |
| pn-sepsis-discovered | 45 | 71 | graphlay fast | 5.5 | 37 | 90 | 0 | 20,094 | 2,795 |
| pn-sepsis-discovered | 45 | 71 | dagre | 26 | 48 | 63 | 7 | 19,121 | 3,008 |
| pn-sepsis-discovered | 45 | 71 | elk | 49 | 57 | 100 | 0 | 20,008 | 2,392 |
| pn-repairexample-im | 48 | 58 | graphlay | 2.6 | 0 | 30 | 0 | 10,729 | 1,994 |
| pn-repairexample-im | 48 | 58 | graphlay fast | 2.6 | 0 | 30 | 0 | 10,729 | 1,994 |
| pn-repairexample-im | 48 | 58 | dagre | 21 | 0 | 38 | 0 | 10,056 | 2,396 |
| pn-repairexample-im | 48 | 58 | elk | 34 | 5 | 58 | 0 | 13,293 | 2,162 |
| dag-50 | 50 | 75 | graphlay | 13 | 18 | 100 | 0 | 18,314 | 2,066 |
| dag-50 | 50 | 75 | graphlay fast | 4.5 | 22 | 112 | 0 | 21,189 | 2,176 |
| dag-50 | 50 | 75 | dagre | 29 | 43 | 78 | 9 | 24,017 | 2,826 |
| dag-50 | 50 | 75 | elk | 45 | 34 | 102 | 0 | 28,786 | 2,735 |
| dag-50-lr | 50 | 75 | graphlay | 19 | 18 | 110 | 0 | 18,024 | 1,917 |
| dag-50-lr | 50 | 75 | graphlay fast | 5.9 | 22 | 111 | 0 | 20,259 | 2,105 |
| dag-50-lr | 50 | 75 | dagre | 39 | 40 | 76 | 13 | 21,527 | 2,791 |
| dag-50-lr | 50 | 75 | elk | 45 | 34 | 102 | 0 | 23,358 | 2,573 |
| pn-pn | 55 | 24 | graphlay | 0.9 | 0 | 7 | 0 | 2,472 | 1,101 |
| pn-pn | 55 | 24 | graphlay fast | 0.9 | 0 | 7 | 0 | 2,472 | 1,101 |
| pn-pn | 55 | 24 | dagre | 7.6 | 0 | 10 | 1 | 2,274 | 4,132 |
| pn-pn | 55 | 24 | elk | 18 | 0 | 12 | 0 | 1,845 | 997 |
| pn-bpic19-alphappp | 62 | 71 | graphlay | 9.7 | 27 | 70 | 0 | 14,147 | 3,066 |
| pn-bpic19-alphappp | 62 | 71 | graphlay fast | 5.7 | 28 | 72 | 0 | 14,113 | 2,865 |
| pn-bpic19-alphappp | 62 | 71 | dagre | 33 | 54 | 58 | 10 | 16,084 | 2,945 |
| pn-bpic19-alphappp | 62 | 71 | elk | 44 | 34 | 76 | 0 | 15,196 | 2,869 |
| pn-rtfm-imf | 64 | 94 | graphlay | 24 | 12 | 93 | 0 | 30,247 | 3,899 |
| pn-rtfm-imf | 64 | 94 | graphlay fast | 10 | 12 | 93 | 0 | 30,247 | 3,899 |
| pn-rtfm-imf | 64 | 94 | dagre | 60 | 18 | 83 | 1 | 32,602 | 4,820 |
| pn-rtfm-imf | 64 | 94 | elk | 40 | 9 | 108 | 0 | 30,879 | 4,145 |
| pn-sepsis-fodina | 64 | 82 | graphlay | 36 | 23 | 95 | 0 | 26,939 | 4,380 |
| pn-sepsis-fodina | 64 | 82 | graphlay fast | 5.1 | 27 | 80 | 0 | 26,673 | 4,317 |
| pn-sepsis-fodina | 64 | 82 | dagre | 36 | 39 | 66 | 8 | 29,636 | 6,323 |
| pn-sepsis-fodina | 64 | 82 | elk | 44 | 21 | 86 | 0 | 27,782 | 3,866 |
| pn-sepsis-imf | 64 | 82 | graphlay | 4.5 | 0 | 60 | 0 | 19,235 | 3,048 |
| pn-sepsis-imf | 64 | 82 | graphlay fast | 4.2 | 0 | 60 | 0 | 19,235 | 3,048 |
| pn-sepsis-imf | 64 | 82 | dagre | 32 | 0 | 56 | 0 | 20,416 | 3,605 |
| pn-sepsis-imf | 64 | 82 | elk | 41 | 0 | 80 | 0 | 20,566 | 3,536 |
| dag-100 | 100 | 150 | graphlay | 37 | 22 | 188 | 0 | 30,232 | 4,459 |
| dag-100 | 100 | 150 | graphlay fast | 9.3 | 22 | 196 | 0 | 30,243 | 4,451 |
| dag-100 | 100 | 150 | dagre | 62 | 76 | 153 | 24 | 56,012 | 7,514 |
| dag-100 | 100 | 150 | elk | 71 | 35 | 198 | 0 | 38,935 | 6,252 |
| dag-200 | 200 | 300 | graphlay | 75 | 63 | 459 | 0 | 96,417 | 14,475 |
| dag-200 | 200 | 300 | graphlay fast | 16 | 84 | 444 | 0 | 87,279 | 11,140 |
| dag-200 | 200 | 300 | dagre | failed | failed | failed | failed | failed | failed |
| dag-200 | 200 | 300 | elk | 153 | 126 | 398 | 0 | 111,783 | 17,935 |
| layered-wide-200x8 | 200 | 600 | graphlay | 544 | 5773 | 1180 | 0 | 801,083 | 22,237 |
| layered-wide-200x8 | 200 | 600 | graphlay fast | 194 | 5868 | 1202 | 0 | 803,582 | 21,541 |
| layered-wide-200x8 | 200 | 600 | dagre | 780 | 7061 | 575 | 1554 | 948,875 | 8,452 |
| layered-wide-200x8 | 200 | 600 | elk | 316 | 7052 | 1072 | 0 | 1,519,145 | 79,800 |
| cycles-300 | 300 | 500 | graphlay | 204 | 187 | 802 | 0 | 191,903 | 33,462 |
| cycles-300 | 300 | 500 | graphlay fast | 80 | 194 | 803 | 0 | 193,004 | 31,744 |
| cycles-300 | 300 | 500 | dagre | 673 | 367 | 533 | 69 | 305,042 | 74,951 |
| cycles-300 | 300 | 500 | elk | 382 | 363 | 744 | 0 | 226,742 | 54,659 |
| dag-400 | 400 | 600 | graphlay | 249 | 170 | 906 | 0 | 216,937 | 35,531 |
| dag-400 | 400 | 600 | graphlay fast | 88 | 187 | 912 | 0 | 211,160 | 30,885 |
| dag-400 | 400 | 600 | dagre | 1103 | 390 | 589 | 117 | 339,318 | 70,277 |
| dag-400 | 400 | 600 | elk | 324 | 318 | 798 | 0 | 258,925 | 58,360 |
| dag-1000 | 1000 | 1500 | graphlay | 599 | 374 | 1829 | 0 | 321,590 | 107,756 |
| dag-1000 | 1000 | 1500 | graphlay fast | 236 | 457 | 1878 | 0 | 326,228 | 147,641 |
| dag-1000 | 1000 | 1500 | dagre | 7134 | 968 | 1475 | 302 | 980,129 | 275,435 |
| dag-1000 | 1000 | 1500 | elk | 1315 | 1051 | 2128 | 0 | 832,087 | 259,290 |
| dag-2000 | 2000 | 3000 | graphlay | 1707 | 753 | 3746 | 0 | 674,339 | 256,335 |
| dag-2000 | 2000 | 3000 | graphlay fast | 761 | 893 | 3743 | 0 | 666,002 | 238,744 |
| dag-2000 | 2000 | 3000 | dagre | failed | failed | failed | failed | failed | failed |
| dag-2000 | 2000 | 3000 | elk | 2950 | 1994 | 4180 | 0 | 1,908,520 | 1,256,460 |
