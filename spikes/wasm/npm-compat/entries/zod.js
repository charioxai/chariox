import { z } from 'zod';

const User = z.object({
  name: z.string().min(2),
  age: z.number().int().positive(),
  email: z.email(),
  tags: z.array(z.string()).max(3),
});

export function run() {
  const ok = User.parse({ name: 'Ada', age: 36, email: 'ada@example.com', tags: ['x'] });
  const bad = User.safeParse({ name: 'A', age: -1, email: 'nope', tags: ['a', 'b', 'c', 'd'] });
  const issues = bad.error.issues.map((i) => `${i.path.join('.')}:${i.code}`).join(',');
  return `ok=${JSON.stringify(ok)} success=${bad.success} issues=${issues}`;
}
