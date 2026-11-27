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

Versions: graphlay 0.5.0, @dagrejs/dagre 3.1.1, elkjs 0.12.0.
Regenerate with `pnpm --filter bench bench`.

## Over the whole corpus

How many cases each engine wins, with a tie counting for every engine that shares it, and the
totals.

| engine | time (ms) wins | crossings wins | bends wins | edges through nodes wins | edge length (px) wins | area (kpx²) wins |
|---|---:|---:|---:|---:|---:|---:|
| graphlay | 6 of 55 | 50 of 55 | 21 of 55 | 53 of 55 | 16 of 55 | 16 of 55 |
| graphlay fast | 47 of 55 | 33 of 55 | 19 of 55 | 53 of 55 | 15 of 55 | 14 of 55 |
| dagre | 0 of 55 | 22 of 55 | 36 of 55 | 30 of 55 | 21 of 55 | 11 of 55 |
| elk | 0 of 55 | 28 of 55 | 6 of 55 | 53 of 55 | 15 of 55 | 28 of 55 |

| engine | total time (ms) | total crossings | total bends | total edges through nodes | total edge length (px) | total area (kpx²) |
|---|---:|---:|---:|---:|---:|---:|
| graphlay | 1966 | 9300 | 7492 | 0 | 2,559,963 | 285,866 |
| graphlay fast | 674 | 9677 | 7620 | 0 | 2,596,037 | 326,949 |
| dagre | 6249 | 12689 | 5514 | 2296 | 3,647,109 | 503,209 |
| elk | 2655 | 12126 | 8476 | 0 | 3,918,776 | 519,560 |

## Case by case

| case | nodes | edges | engine | time (ms) | crossings | bends | edges through nodes | edge length (px) | area (kpx²) |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|
| bundled-lane-off-outline | 3 | 7 | graphlay | 0.5 | 0 | 2 | 0 | 1,315 | 55 |
| bundled-lane-off-outline | 3 | 7 | graphlay fast | 0.4 | 0 | 2 | 0 | 1,315 | 55 |
| bundled-lane-off-outline | 3 | 7 | dagre | 2.2 | 0 | 8 | 0 | 1,159 | 55 |
| bundled-lane-off-outline | 3 | 7 | elk | 9.5 | 0 | 20 | 0 | 1,468 | 49 |
| edge-label-gap-reservation | 3 | 3 | graphlay | 0.4 | 0 | 4 | 0 | 464 | 41 |
| edge-label-gap-reservation | 3 | 3 | graphlay fast | 0.3 | 0 | 4 | 0 | 464 | 41 |
| edge-label-gap-reservation | 3 | 3 | dagre | 0.9 | 0 | 4 | 0 | 400 | 42 |
| edge-label-gap-reservation | 3 | 3 | elk | 5.6 | 0 | 4 | 0 | 433 | 30 |
| dfg-small-example-all | 5 | 6 | graphlay | 0.4 | 0 | 6 | 0 | 828 | 123 |
| dfg-small-example-all | 5 | 6 | graphlay fast | 0.3 | 0 | 6 | 0 | 828 | 123 |
| dfg-small-example-all | 5 | 6 | dagre | 1.8 | 0 | 5 | 0 | 770 | 151 |
| dfg-small-example-all | 5 | 6 | elk | 8.4 | 0 | 6 | 0 | 804 | 119 |
| dfg-small-example-all-diag | 5 | 6 | graphlay | 0.3 | 0 | 3 | 0 | 659 | 123 |
| dfg-small-example-all-diag | 5 | 6 | graphlay fast | 0.3 | 0 | 3 | 0 | 659 | 123 |
| dfg-small-example-all-diag | 5 | 6 | dagre | 1.3 | 0 | 5 | 0 | 770 | 151 |
| dfg-small-example-all-diag | 5 | 6 | elk | 6.4 | 0 | 6 | 0 | 804 | 119 |
| dfg-small-example-min5pct | 5 | 6 | graphlay | 0.3 | 0 | 6 | 0 | 828 | 123 |
| dfg-small-example-min5pct | 5 | 6 | graphlay fast | 0.3 | 0 | 6 | 0 | 828 | 123 |
| dfg-small-example-min5pct | 5 | 6 | dagre | 1.3 | 0 | 5 | 0 | 770 | 151 |
| dfg-small-example-min5pct | 5 | 6 | elk | 7.1 | 0 | 6 | 0 | 804 | 119 |
| dfg-small-example-top25pct | 5 | 5 | graphlay | 0.3 | 0 | 6 | 0 | 644 | 128 |
| dfg-small-example-top25pct | 5 | 5 | graphlay fast | 0.3 | 0 | 6 | 0 | 644 | 128 |
| dfg-small-example-top25pct | 5 | 5 | dagre | 0.9 | 0 | 5 | 0 | 763 | 152 |
| dfg-small-example-top25pct | 5 | 5 | elk | 5.4 | 0 | 6 | 0 | 818 | 131 |
| dfg-an1-all | 8 | 18 | graphlay | 1.0 | 0 | 30 | 0 | 4,878 | 349 |
| dfg-an1-all | 8 | 18 | graphlay fast | 1.1 | 0 | 30 | 0 | 4,878 | 349 |
| dfg-an1-all | 8 | 18 | dagre | 5.9 | 5 | 24 | 0 | 3,718 | 318 |
| dfg-an1-all | 8 | 18 | elk | 13 | 3 | 34 | 0 | 3,538 | 216 |
| dfg-an1-all-diag | 8 | 18 | graphlay | 1.1 | 0 | 15 | 0 | 4,032 | 349 |
| dfg-an1-all-diag | 8 | 18 | graphlay fast | 1.1 | 0 | 15 | 0 | 4,032 | 349 |
| dfg-an1-all-diag | 8 | 18 | dagre | 5.9 | 5 | 24 | 0 | 3,718 | 318 |
| dfg-an1-all-diag | 8 | 18 | elk | 12 | 3 | 34 | 0 | 3,538 | 216 |
| dfg-an1-min5pct | 8 | 18 | graphlay | 0.9 | 0 | 30 | 0 | 4,878 | 349 |
| dfg-an1-min5pct | 8 | 18 | graphlay fast | 0.9 | 0 | 30 | 0 | 4,878 | 349 |
| dfg-an1-min5pct | 8 | 18 | dagre | 5.1 | 5 | 24 | 0 | 3,718 | 318 |
| dfg-an1-min5pct | 8 | 18 | elk | 11 | 3 | 34 | 0 | 3,538 | 216 |
| dfg-an1-top25pct | 8 | 6 | graphlay | 0.2 | 0 | 2 | 0 | 520 | 212 |
| dfg-an1-top25pct | 8 | 6 | graphlay fast | 0.2 | 0 | 2 | 0 | 520 | 212 |
| dfg-an1-top25pct | 8 | 6 | dagre | 1.2 | 0 | 2 | 0 | 443 | 222 |
| dfg-an1-top25pct | 8 | 6 | elk | 5.4 | 0 | 2 | 0 | 518 | 260 |
| parallel-selfloops | 8 | 29 | graphlay | 0.4 | 0 | 16 | 0 | 1,662 | 107 |
| parallel-selfloops | 8 | 29 | graphlay fast | 0.3 | 0 | 16 | 0 | 1,662 | 107 |
| parallel-selfloops | 8 | 29 | dagre | 3.7 | 0 | 22 | 0 | 2,717 | 169 |
| parallel-selfloops | 8 | 29 | elk | 7.5 | 0 | 16 | 0 | 1,433 | 103 |
| multihop-run-through-rank | 9 | 8 | graphlay | 0.3 | 0 | 6 | 0 | 975 | 201 |
| multihop-run-through-rank | 9 | 8 | graphlay fast | 0.3 | 0 | 6 | 0 | 975 | 201 |
| multihop-run-through-rank | 9 | 8 | dagre | 1.9 | 1 | 6 | 0 | 832 | 198 |
| multihop-run-through-rank | 9 | 8 | elk | 5.6 | 0 | 8 | 0 | 1,162 | 234 |
| chain-10 | 10 | 9 | graphlay | 0.3 | 0 | 0 | 0 | 486 | 121 |
| chain-10 | 10 | 9 | graphlay fast | 0.3 | 0 | 0 | 0 | 486 | 121 |
| chain-10 | 10 | 9 | dagre | 1.7 | 0 | 0 | 0 | 486 | 121 |
| chain-10 | 10 | 9 | elk | 5.4 | 0 | 0 | 0 | 486 | 121 |
| chain-10-lr | 10 | 9 | graphlay | 0.3 | 0 | 0 | 0 | 486 | 88 |
| chain-10-lr | 10 | 9 | graphlay fast | 0.2 | 0 | 0 | 0 | 486 | 88 |
| chain-10-lr | 10 | 9 | dagre | 1.7 | 0 | 0 | 0 | 486 | 88 |
| chain-10-lr | 10 | 9 | elk | 5.0 | 0 | 0 | 0 | 486 | 88 |
| dfg-repairexample-all | 10 | 28 | graphlay | 3.0 | 4 | 51 | 0 | 7,560 | 569 |
| dfg-repairexample-all | 10 | 28 | graphlay fast | 1.7 | 5 | 54 | 0 | 8,604 | 575 |
| dfg-repairexample-all | 10 | 28 | dagre | 7.7 | 20 | 40 | 0 | 7,824 | 548 |
| dfg-repairexample-all | 10 | 28 | elk | 12 | 6 | 54 | 0 | 5,034 | 342 |
| dfg-repairexample-all-diag | 10 | 28 | graphlay | 3.1 | 4 | 28 | 0 | 6,488 | 569 |
| dfg-repairexample-all-diag | 10 | 28 | graphlay fast | 1.7 | 7 | 29 | 0 | 7,084 | 575 |
| dfg-repairexample-all-diag | 10 | 28 | dagre | 8.0 | 20 | 40 | 0 | 7,824 | 548 |
| dfg-repairexample-all-diag | 10 | 28 | elk | 14 | 6 | 54 | 0 | 5,034 | 342 |
| dfg-repairexample-min5pct | 10 | 26 | graphlay | 2.0 | 4 | 40 | 0 | 6,308 | 521 |
| dfg-repairexample-min5pct | 10 | 26 | graphlay fast | 1.1 | 4 | 40 | 0 | 6,308 | 521 |
| dfg-repairexample-min5pct | 10 | 26 | dagre | 6.5 | 18 | 32 | 0 | 7,087 | 538 |
| dfg-repairexample-min5pct | 10 | 26 | elk | 11 | 4 | 44 | 0 | 4,934 | 359 |
| dfg-repairexample-top25pct | 10 | 11 | graphlay | 0.4 | 0 | 12 | 0 | 1,483 | 302 |
| dfg-repairexample-top25pct | 10 | 11 | graphlay fast | 0.4 | 0 | 12 | 0 | 1,483 | 302 |
| dfg-repairexample-top25pct | 10 | 11 | dagre | 2.4 | 3 | 10 | 1 | 1,979 | 348 |
| dfg-repairexample-top25pct | 10 | 11 | elk | 6.6 | 0 | 14 | 0 | 1,688 | 279 |
| pn-small-example-im | 11 | 12 | graphlay | 0.4 | 0 | 4 | 0 | 1,343 | 187 |
| pn-small-example-im | 11 | 12 | graphlay fast | 0.4 | 0 | 4 | 0 | 1,343 | 187 |
| pn-small-example-im | 11 | 12 | dagre | 2.7 | 0 | 9 | 0 | 1,548 | 195 |
| pn-small-example-im | 11 | 12 | elk | 6.8 | 0 | 12 | 0 | 2,117 | 196 |
| bk-infeasible-layer | 12 | 11 | graphlay | 0.4 | 0 | 12 | 0 | 1,886 | 215 |
| bk-infeasible-layer | 12 | 11 | graphlay fast | 0.4 | 0 | 12 | 0 | 1,886 | 215 |
| bk-infeasible-layer | 12 | 11 | dagre | 2.1 | 0 | 5 | 0 | 1,154 | 257 |
| bk-infeasible-layer | 12 | 11 | elk | 6.1 | 0 | 12 | 0 | 1,894 | 296 |
| pn-repairexample-alphappp | 12 | 5 | graphlay | 0.2 | 0 | 2 | 0 | 486 | 289 |
| pn-repairexample-alphappp | 12 | 5 | graphlay fast | 0.2 | 0 | 2 | 0 | 486 | 289 |
| pn-repairexample-alphappp | 12 | 5 | dagre | 1.2 | 0 | 2 | 0 | 446 | 395 |
| pn-repairexample-alphappp | 12 | 5 | elk | 5.5 | 0 | 2 | 0 | 351 | 159 |
| dfg-rtfm-all | 13 | 78 | graphlay | 71 | 121 | 155 | 0 | 62,759 | 2,342 |
| dfg-rtfm-all | 13 | 78 | graphlay fast | 24 | 133 | 201 | 0 | 73,332 | 2,800 |
| dfg-rtfm-all | 13 | 78 | dagre | 37 | 272 | 123 | 2 | 54,564 | 1,804 |
| dfg-rtfm-all | 13 | 78 | elk | 48 | 226 | 226 | 0 | 54,668 | 1,389 |
| dfg-rtfm-all-diag | 13 | 78 | graphlay | 74 | 128 | 71 | 0 | 54,888 | 2,391 |
| dfg-rtfm-all-diag | 13 | 78 | graphlay fast | 23 | 140 | 86 | 0 | 61,124 | 2,778 |
| dfg-rtfm-all-diag | 13 | 78 | dagre | 37 | 272 | 123 | 2 | 54,564 | 1,804 |
| dfg-rtfm-all-diag | 13 | 78 | elk | 51 | 226 | 226 | 0 | 54,668 | 1,389 |
| dfg-rtfm-min5pct | 13 | 14 | graphlay | 0.5 | 0 | 16 | 0 | 5,613 | 1,278 |
| dfg-rtfm-min5pct | 13 | 14 | graphlay fast | 0.5 | 0 | 16 | 0 | 5,613 | 1,278 |
| dfg-rtfm-min5pct | 13 | 14 | dagre | 3.5 | 0 | 10 | 0 | 5,016 | 1,113 |
| dfg-rtfm-min5pct | 13 | 14 | elk | 6.5 | 0 | 18 | 0 | 6,670 | 1,198 |
| dfg-rtfm-top25pct | 13 | 26 | graphlay | 2.0 | 0 | 30 | 0 | 7,064 | 852 |
| dfg-rtfm-top25pct | 13 | 26 | graphlay fast | 1.5 | 1 | 28 | 0 | 6,522 | 902 |
| dfg-rtfm-top25pct | 13 | 26 | dagre | 7.4 | 4 | 35 | 0 | 7,684 | 1,009 |
| dfg-rtfm-top25pct | 13 | 26 | elk | 13 | 1 | 46 | 0 | 7,193 | 864 |
| pn-bpic12-tsinghua | 16 | 28 | graphlay | 3.8 | 6 | 37 | 0 | 7,783 | 673 |
| pn-bpic12-tsinghua | 16 | 28 | graphlay fast | 2.1 | 6 | 37 | 0 | 7,783 | 673 |
| pn-bpic12-tsinghua | 16 | 28 | dagre | 8.7 | 11 | 33 | 2 | 7,454 | 668 |
| pn-bpic12-tsinghua | 16 | 28 | elk | 12 | 17 | 52 | 0 | 6,705 | 556 |
| diamond-blocks | 17 | 24 | graphlay | 0.5 | 0 | 32 | 0 | 4,560 | 407 |
| diamond-blocks | 17 | 24 | graphlay fast | 0.5 | 0 | 32 | 0 | 4,560 | 407 |
| diamond-blocks | 17 | 24 | dagre | 3.6 | 0 | 16 | 0 | 2,644 | 407 |
| diamond-blocks | 17 | 24 | elk | 6.8 | 0 | 32 | 0 | 4,016 | 774 |
| typegraph-ocel2-p2p | 17 | 34 | graphlay | 8.5 | 15 | 16 | 0 | 13,971 | 1,277 |
| typegraph-ocel2-p2p | 17 | 34 | graphlay fast | 1.9 | 17 | 18 | 0 | 13,966 | 1,307 |
| typegraph-ocel2-p2p | 17 | 34 | dagre | 9.4 | 55 | 48 | 1 | 19,452 | 1,282 |
| typegraph-ocel2-p2p | 17 | 34 | elk | 20 | 15 | 60 | 0 | 16,076 | 1,397 |
| typegraph-order-management | 17 | 46 | graphlay | 17 | 76 | 40 | 0 | 29,965 | 1,492 |
| typegraph-order-management | 17 | 46 | graphlay fast | 3.0 | 99 | 40 | 0 | 29,003 | 1,490 |
| typegraph-order-management | 17 | 46 | dagre | 13 | 142 | 67 | 4 | 27,897 | 1,412 |
| typegraph-order-management | 17 | 46 | elk | 22 | 88 | 116 | 0 | 39,829 | 1,322 |
| dfg-sepsis-all | 18 | 135 | graphlay | 257 | 532 | 366 | 0 | 164,688 | 5,331 |
| dfg-sepsis-all | 18 | 135 | graphlay fast | 55 | 570 | 372 | 0 | 167,732 | 5,568 |
| dfg-sepsis-all | 18 | 135 | dagre | 81 | 809 | 227 | 2 | 132,040 | 3,247 |
| dfg-sepsis-all | 18 | 135 | elk | 131 | 652 | 390 | 0 | 146,701 | 3,618 |
| dfg-sepsis-all-diag | 18 | 135 | graphlay | 263 | 584 | 157 | 0 | 142,215 | 5,272 |
| dfg-sepsis-all-diag | 18 | 135 | graphlay fast | 49 | 648 | 154 | 0 | 144,491 | 5,523 |
| dfg-sepsis-all-diag | 18 | 135 | dagre | 71 | 809 | 227 | 2 | 132,040 | 3,247 |
| dfg-sepsis-all-diag | 18 | 135 | elk | 125 | 652 | 390 | 0 | 146,701 | 3,618 |
| dfg-sepsis-min5pct | 18 | 50 | graphlay | 21 | 20 | 96 | 0 | 26,837 | 2,430 |
| dfg-sepsis-min5pct | 18 | 50 | graphlay fast | 9.9 | 20 | 94 | 0 | 27,566 | 2,228 |
| dfg-sepsis-min5pct | 18 | 50 | dagre | 20 | 69 | 73 | 0 | 26,319 | 1,939 |
| dfg-sepsis-min5pct | 18 | 50 | elk | 32 | 21 | 110 | 0 | 23,859 | 2,079 |
| dfg-sepsis-top25pct | 18 | 49 | graphlay | 22 | 19 | 92 | 0 | 26,223 | 2,372 |
| dfg-sepsis-top25pct | 18 | 49 | graphlay fast | 7.2 | 19 | 92 | 0 | 26,327 | 2,199 |
| dfg-sepsis-top25pct | 18 | 49 | dagre | 19 | 62 | 72 | 0 | 25,392 | 1,932 |
| dfg-sepsis-top25pct | 18 | 49 | elk | 32 | 27 | 114 | 0 | 28,503 | 1,929 |
| dag-20 | 20 | 30 | graphlay | 3.4 | 5 | 44 | 0 | 5,976 | 759 |
| dag-20 | 20 | 30 | graphlay fast | 1.1 | 5 | 44 | 0 | 5,976 | 759 |
| dag-20 | 20 | 30 | dagre | 7.0 | 7 | 33 | 2 | 7,307 | 835 |
| dag-20 | 20 | 30 | elk | 13 | 5 | 40 | 0 | 6,247 | 600 |
| typegraph-container-logistics | 21 | 34 | graphlay | 5.2 | 28 | 15 | 0 | 15,969 | 1,477 |
| typegraph-container-logistics | 21 | 34 | graphlay fast | 2.1 | 31 | 17 | 0 | 14,083 | 1,436 |
| typegraph-container-logistics | 21 | 34 | dagre | 8.9 | 73 | 41 | 1 | 17,940 | 1,464 |
| typegraph-container-logistics | 21 | 34 | elk | 15 | 23 | 66 | 0 | 17,659 | 1,299 |
| wide-shallow-30 | 31 | 30 | graphlay | 0.8 | 0 | 56 | 0 | 46,160 | 1,766 |
| wide-shallow-30 | 31 | 30 | graphlay fast | 0.8 | 0 | 56 | 0 | 46,160 | 1,766 |
| wide-shallow-30 | 31 | 30 | dagre | 4.7 | 0 | 30 | 0 | 36,454 | 780 |
| wide-shallow-30 | 31 | 30 | elk | 9.7 | 0 | 58 | 0 | 79,526 | 1,993 |
| grid-6x6 | 36 | 180 | graphlay | 15 | 1095 | 380 | 0 | 94,344 | 1,541 |
| grid-6x6 | 36 | 180 | graphlay fast | 11 | 1095 | 380 | 0 | 94,344 | 1,541 |
| grid-6x6 | 36 | 180 | dagre | 39 | 923 | 152 | 162 | 197,807 | 2,514 |
| grid-6x6 | 36 | 180 | elk | 42 | 1135 | 380 | 0 | 123,554 | 4,492 |
| hub-spoke-40 | 41 | 40 | graphlay | 2.4 | 0 | 38 | 0 | 34,513 | 1,521 |
| hub-spoke-40 | 41 | 40 | graphlay fast | 2.5 | 0 | 38 | 0 | 34,513 | 1,521 |
| hub-spoke-40 | 41 | 40 | dagre | 9.0 | 0 | 40 | 0 | 30,521 | 1,378 |
| hub-spoke-40 | 41 | 40 | elk | 12 | 0 | 76 | 0 | 48,290 | 2,185 |
| pn-rtfm-imf02-reference | 42 | 52 | graphlay | 1.7 | 0 | 26 | 0 | 9,150 | 1,485 |
| pn-rtfm-imf02-reference | 42 | 52 | graphlay fast | 1.6 | 0 | 26 | 0 | 9,150 | 1,485 |
| pn-rtfm-imf02-reference | 42 | 52 | dagre | 11 | 0 | 39 | 0 | 8,290 | 1,614 |
| pn-rtfm-imf02-reference | 42 | 52 | elk | 13 | 0 | 44 | 0 | 9,481 | 1,229 |
| pn-sepsis-discovered | 45 | 71 | graphlay | 13 | 36 | 89 | 0 | 21,809 | 3,110 |
| pn-sepsis-discovered | 45 | 71 | graphlay fast | 4.7 | 36 | 89 | 0 | 21,809 | 3,110 |
| pn-sepsis-discovered | 45 | 71 | dagre | 18 | 48 | 63 | 7 | 19,121 | 3,008 |
| pn-sepsis-discovered | 45 | 71 | elk | 28 | 57 | 100 | 0 | 20,008 | 2,392 |
| pn-repairexample-im | 48 | 58 | graphlay | 1.8 | 0 | 28 | 0 | 9,314 | 2,072 |
| pn-repairexample-im | 48 | 58 | graphlay fast | 2.0 | 0 | 28 | 0 | 9,314 | 2,072 |
| pn-repairexample-im | 48 | 58 | dagre | 16 | 0 | 38 | 0 | 10,056 | 2,396 |
| pn-repairexample-im | 48 | 58 | elk | 27 | 5 | 58 | 0 | 13,293 | 2,162 |
| dag-50 | 50 | 75 | graphlay | 10 | 18 | 92 | 0 | 17,663 | 1,937 |
| dag-50 | 50 | 75 | graphlay fast | 3.3 | 22 | 112 | 0 | 21,721 | 2,283 |
| dag-50 | 50 | 75 | dagre | 19 | 43 | 78 | 9 | 24,017 | 2,826 |
| dag-50 | 50 | 75 | elk | 29 | 34 | 102 | 0 | 28,786 | 2,735 |
| dag-50-lr | 50 | 75 | graphlay | 11 | 18 | 112 | 0 | 19,086 | 2,143 |
| dag-50-lr | 50 | 75 | graphlay fast | 3.2 | 22 | 105 | 0 | 20,664 | 2,423 |
| dag-50-lr | 50 | 75 | dagre | 21 | 40 | 76 | 13 | 21,527 | 2,791 |
| dag-50-lr | 50 | 75 | elk | 29 | 34 | 102 | 0 | 23,358 | 2,573 |
| pn-pn | 55 | 24 | graphlay | 0.7 | 0 | 9 | 0 | 2,444 | 1,101 |
| pn-pn | 55 | 24 | graphlay fast | 0.7 | 0 | 9 | 0 | 2,444 | 1,101 |
| pn-pn | 55 | 24 | dagre | 5.2 | 0 | 10 | 1 | 2,274 | 4,132 |
| pn-pn | 55 | 24 | elk | 13 | 0 | 12 | 0 | 1,845 | 997 |
| pn-bpic19-alphappp | 62 | 71 | graphlay | 10 | 27 | 67 | 0 | 15,935 | 3,071 |
| pn-bpic19-alphappp | 62 | 71 | graphlay fast | 4.8 | 28 | 67 | 0 | 15,231 | 2,899 |
| pn-bpic19-alphappp | 62 | 71 | dagre | 21 | 54 | 58 | 10 | 16,084 | 2,945 |
| pn-bpic19-alphappp | 62 | 71 | elk | 29 | 34 | 76 | 0 | 15,196 | 2,869 |
| pn-rtfm-imf | 64 | 94 | graphlay | 18 | 12 | 90 | 0 | 30,896 | 4,132 |
| pn-rtfm-imf | 64 | 94 | graphlay fast | 7.5 | 12 | 90 | 0 | 30,896 | 4,132 |
| pn-rtfm-imf | 64 | 94 | dagre | 30 | 18 | 83 | 1 | 32,602 | 4,820 |
| pn-rtfm-imf | 64 | 94 | elk | 35 | 9 | 108 | 0 | 30,879 | 4,145 |
| pn-sepsis-fodina | 64 | 82 | graphlay | 22 | 21 | 85 | 0 | 27,754 | 4,651 |
| pn-sepsis-fodina | 64 | 82 | graphlay fast | 6.4 | 29 | 84 | 0 | 28,263 | 4,510 |
| pn-sepsis-fodina | 64 | 82 | dagre | 26 | 39 | 66 | 8 | 29,636 | 6,323 |
| pn-sepsis-fodina | 64 | 82 | elk | 35 | 21 | 86 | 0 | 27,782 | 3,866 |
| pn-sepsis-imf | 64 | 82 | graphlay | 3.2 | 0 | 53 | 0 | 18,733 | 3,239 |
| pn-sepsis-imf | 64 | 82 | graphlay fast | 3.2 | 0 | 53 | 0 | 18,733 | 3,239 |
| pn-sepsis-imf | 64 | 82 | dagre | 23 | 0 | 56 | 0 | 20,416 | 3,605 |
| pn-sepsis-imf | 64 | 82 | elk | 28 | 0 | 80 | 0 | 20,566 | 3,536 |
| dag-100 | 100 | 150 | graphlay | 24 | 22 | 174 | 0 | 31,013 | 4,624 |
| dag-100 | 100 | 150 | graphlay fast | 6.1 | 22 | 190 | 0 | 30,904 | 4,593 |
| dag-100 | 100 | 150 | dagre | 45 | 76 | 153 | 24 | 56,012 | 7,514 |
| dag-100 | 100 | 150 | elk | 56 | 35 | 198 | 0 | 38,935 | 6,252 |
| dag-200 | 200 | 300 | graphlay | 52 | 63 | 459 | 0 | 100,334 | 15,490 |
| dag-200 | 200 | 300 | graphlay fast | 15 | 85 | 452 | 0 | 87,547 | 11,672 |
| dag-200 | 200 | 300 | dagre | failed | failed | failed | failed | failed | failed |
| dag-200 | 200 | 300 | elk | 103 | 126 | 398 | 0 | 111,783 | 17,935 |
| layered-wide-200x8 | 200 | 600 | graphlay | 390 | 5773 | 1170 | 0 | 808,452 | 23,468 |
| layered-wide-200x8 | 200 | 600 | graphlay fast | 157 | 5868 | 1194 | 0 | 810,995 | 21,119 |
| layered-wide-200x8 | 200 | 600 | dagre | 465 | 7061 | 575 | 1554 | 948,875 | 8,452 |
| layered-wide-200x8 | 200 | 600 | elk | 233 | 7052 | 1072 | 0 | 1,519,145 | 79,800 |
| cycles-300 | 300 | 500 | graphlay | 133 | 189 | 806 | 0 | 198,568 | 35,782 |
| cycles-300 | 300 | 500 | graphlay fast | 49 | 199 | 809 | 0 | 199,106 | 34,034 |
| cycles-300 | 300 | 500 | dagre | 331 | 367 | 533 | 69 | 305,042 | 74,951 |
| cycles-300 | 300 | 500 | elk | 225 | 363 | 744 | 0 | 226,742 | 54,659 |
| dag-400 | 400 | 600 | graphlay | 142 | 169 | 948 | 0 | 222,092 | 35,062 |
| dag-400 | 400 | 600 | graphlay fast | 49 | 182 | 930 | 0 | 219,664 | 34,187 |
| dag-400 | 400 | 600 | dagre | 631 | 390 | 589 | 117 | 339,318 | 70,277 |
| dag-400 | 400 | 600 | elk | 218 | 318 | 798 | 0 | 258,925 | 58,360 |
| dag-1000 | 1000 | 1500 | graphlay | 404 | 374 | 1827 | 0 | 335,321 | 121,785 |
| dag-1000 | 1000 | 1500 | graphlay fast | 169 | 457 | 1850 | 0 | 344,204 | 166,553 |
| dag-1000 | 1000 | 1500 | dagre | 4141 | 968 | 1475 | 302 | 980,129 | 275,435 |
| dag-1000 | 1000 | 1500 | elk | 895 | 1051 | 2128 | 0 | 832,087 | 259,290 |
| dag-2000 | 2000 | 3000 | graphlay | 1058 | 753 | 3702 | 0 | 705,252 | 246,101 |
| dag-2000 | 2000 | 3000 | graphlay fast | 560 | 894 | 3712 | 0 | 699,875 | 218,023 |
| dag-2000 | 2000 | 3000 | dagre | failed | failed | failed | failed | failed | failed |
| dag-2000 | 2000 | 3000 | elk | 1806 | 1994 | 4180 | 0 | 1,908,520 | 1,256,460 |
