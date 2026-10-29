# syntax=docker/dockerfile:1
FROM node:22.17.1-bookworm@sha256:37ff334612f77d8f999c10af8797727b731629c26f2e83caa6af390998bdc49c AS build
WORKDIR /opt/chariox
RUN corepack enable && corepack prepare pnpm@9.15.0 --activate
COPY . .
RUN pnpm install --frozen-lockfile && pnpm --filter @chariox/server run build

FROM node:22.17.1-bookworm-slim@sha256:2fa754a9ba4d7adbd2a51d182eaabbe355c82b673624035a38c0d42b08724854
ENV NODE_ENV=production HOST=0.0.0.0 PORT=43110
WORKDIR /opt/chariox
COPY --from=build --chown=node:node /opt/chariox /opt/chariox
USER node
EXPOSE 43110
CMD ["node", "apps/server/dist/hosted-agent-trigger-main.js"]
