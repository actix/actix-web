# Actix Ecosystem Dependency Graphs

See rendered versions of these dot graphs [on the wiki](https://github.com/actix/actix-web/wiki/Dependency-Graph).

## Rendering

Dot graphs were rendered using the `dot` command from [GraphViz](https://www.graphviz.org/doc/info/command.html):

```sh
for f in $(ls docs/graphs/*.dot | xargs); do dot $f -Tpng -o${f:r}.png; done
```
