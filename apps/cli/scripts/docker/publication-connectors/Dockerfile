FROM node:22-alpine

RUN apk add --no-cache ca-certificates curl openssl

WORKDIR /client
RUN npm init -y >/dev/null && npm install ws@8.18.3 >/dev/null

COPY client.mjs /client/client.mjs

ENTRYPOINT ["node", "/client/client.mjs"]
