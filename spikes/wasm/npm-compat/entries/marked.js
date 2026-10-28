import { marked } from 'marked';

const doc = `# Title

Some *emphasis*, **strong** and \`code\`.

- one
- two [link](https://example.com)

| a | b |
|---|---|
| 1 | 2 |

\`\`\`js
const x = 1;
\`\`\`
`;

export function run() {
  return marked.parse(doc, { async: false });
}
