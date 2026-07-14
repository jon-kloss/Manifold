# Third-party content notices

FICSIT Planner bundles the following community-sourced assets. Satisfactory,
its world, and all in-game content are © Coffee Stain Studios AB; these assets
are derived from the game and are used here as a non-commercial planning aid.

## World map render — `renderer/public/map/world.webp`

In-game world map render (5000×5000) from the
[satisfactory_node_heatmap](https://github.com/Hirashi3630/satisfactory_node_heatmap)
project by Hirashi3630, MIT license. The same render family is used by
community tools such as satisfactory-calculator.com. Calibration follows the
community-standard map bounds: X −324,698.83…+425,301.83 cm,
Y −375,000…+375,000 cm (image top = north).

## Resource node dataset — `crates/gamedata/assets/vendor/nodes_vanilla.json`

Vanilla resource-node positions/purities from the same
[satisfactory_node_heatmap](https://github.com/Hirashi3630/satisfactory_node_heatmap)
repository (MIT), extracted in-game via FicsIt-Networking and cross-checked
against satisfactory-calculator.com's 1.1 map data.
`crates/gamedata/assets/world-nodes.json` is generated from it by
`scripts/gen-world-nodes.py`.

### MIT license (Hirashi3630/satisfactory_node_heatmap)

> Permission is hereby granted, free of charge, to any person obtaining a copy
> of this software and associated documentation files (the "Software"), to deal
> in the Software without restriction, including without limitation the rights
> to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
> copies of the Software, and to permit persons to whom the Software is
> furnished to do so, subject to the following conditions:
>
> The above copyright notice and this permission notice shall be included in
> all copies or substantial portions of the Software.
>
> THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
> IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
> FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
> AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
> LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING
> FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS
> IN THE SOFTWARE.
