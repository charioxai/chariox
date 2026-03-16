import Fastify from 'fastify';

export const buildServer = () => {
  const app = Fastify({ logger: false });

  app.get('/health', async () => {
    return { status: 'ok' };
  });

  return app;
};

if (import.meta.url === `file://${process.argv[1]}`) {
  const app = buildServer();
  const host = process.env.HOST ?? '0.0.0.0';
  const port = Number(process.env.PORT ?? 3000);

  app
    .listen({ host, port })
    .catch((error) => {
      app.log.error(error);
      process.exit(1);
    });
}
