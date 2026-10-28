import { groupBy, sortBy, sumBy, uniq, chunk, mapValues, kebabCase } from 'lodash-es';

const items = [
  { name: 'Pear', kind: 'fruit', price: 3 },
  { name: 'Leek', kind: 'veg', price: 2 },
  { name: 'Apple', kind: 'fruit', price: 1 },
  { name: 'Kale', kind: 'veg', price: 4 },
  { name: 'Fig', kind: 'fruit', price: 5 },
];

export function run() {
  const groups = mapValues(groupBy(items, 'kind'), (g) => sortBy(g, 'price').map((i) => i.name));
  return JSON.stringify({
    groups,
    total: sumBy(items, 'price'),
    kinds: uniq(items.map((i) => i.kind)),
    chunks: chunk(items.map((i) => i.price), 2),
    slug: kebabCase('Hello World FromLodash'),
  });
}
