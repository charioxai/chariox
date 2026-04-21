import { createHmac, createPrivateKey, sign } from 'node:crypto'
import { WebSocket } from 'ws'

function requiredEnv(name) {
  const value = process.env[name]
  if (!value) throw new Error(`${name} is required`)
  return value
}

function hasAcceptedRunMetadata(body) {
  return !!body && (body.workflow_run?.id || body.queued === true)
}

function slackHeaders(secret, body, contentType = 'application/json') {
  const timestamp = String(Math.floor(Date.now() / 1000))
  return {
    'content-type': contentType,
    'x-slack-request-timestamp': timestamp,
    'x-slack-signature': `v0=${createHmac('sha256', secret).update(`v0:${timestamp}:${body}`).digest('hex')}`,
  }
}

function discordHeaders(privateKeyPem, body) {
  const timestamp = String(Math.floor(Date.now() / 1000))
  const privateKey = createPrivateKey(privateKeyPem)
  return {
    'content-type': 'application/json',
    'x-signature-timestamp': timestamp,
    'x-signature-ed25519': sign(null, Buffer.from(`${timestamp}${body}`), privateKey).toString('hex'),
  }
}

function whatsAppHeaders(secret, body) {
  return {
    'content-type': 'application/json',
    'x-hub-signature-256': `sha256=${createHmac('sha256', secret).update(body).digest('hex')}`,
  }
}

function whatsAppPayload() {
  return {
    object: 'whatsapp_business_account',
    entry: [{
      id: 'business-id',
      changes: [{
        field: 'messages',
        value: {
          messaging_product: 'whatsapp',
          metadata: {
            display_phone_number: '15557654321',
            phone_number_id: 'phone-number-id',
          },
          contacts: [{ wa_id: '15551234567', profile: { name: 'Docker Drill' } }],
          messages: [{ from: '15551234567', id: 'wamid.docker', text: { body: 'docker-publication' }, type: 'text' }],
        },
      }],
    }],
  }
}

async function expectAccepted(response, label) {
  const text = await response.text()
  let body = null
  try {
    body = text ? JSON.parse(text) : {}
  } catch {
    throw new Error(`${label} returned non-JSON ${response.status}: ${text}`)
  }
  if (response.status !== 202 || !body.accepted || !hasAcceptedRunMetadata(body)) {
    throw new Error(`${label} expected HTTP 202 accepted run metadata, got ${response.status}: ${text}`)
  }
  return body
}

async function invokeWebSocket(url) {
  const socket = new WebSocket(url, { rejectUnauthorized: false })
  const messages = []
  const waiters = []
  let socketError = null
  socket.on('message', (data) => {
    const parsed = JSON.parse(data.toString())
    const waiter = waiters.shift()
    if (waiter) waiter(parsed)
    else messages.push(parsed)
  })
  socket.on('error', (error) => {
    socketError = error
  })
  const read = async () => {
    if (socketError) throw socketError
    const existing = messages.shift()
    if (existing) return existing
    return await new Promise((resolve, reject) => {
      const timeout = setTimeout(() => reject(new Error('timed out waiting for websocket message')), 20_000)
      waiters.push((message) => {
        clearTimeout(timeout)
        resolve(message)
      })
    })
  }
  try {
    const ready = await read()
    if (ready.type !== 'ready') throw new Error(`expected ready message, got ${JSON.stringify(ready)}`)
    socket.send(JSON.stringify({ type: 'invoke', input: { task: 'docker-websocket' } }))
    const accepted = await read()
    if (accepted.type !== 'accepted' || (!accepted.workflow_run?.id && !accepted.queued)) {
      throw new Error(`expected accepted message, got ${JSON.stringify(accepted)}`)
    }
    return accepted
  } finally {
    socket.close()
  }
}

async function main() {
  const connector = requiredEnv('CONNECTOR')
  const baseUrl = requiredEnv('BASE_URL')
  let result

  if (connector === 'http' || connector === 'https') {
    result = await expectAccepted(await fetch(`${baseUrl}/docker/${connector}-publication`), connector)
  } else if (connector === 'ws' || connector === 'wss') {
    result = await invokeWebSocket(`${baseUrl}/.well-known/arroba/publication/ws`)
  } else if (connector === 'slack') {
    const body = new URLSearchParams({
      team_id: 'T-DOCKER',
      user_id: 'U-DOCKER',
      command: '/arroba',
      text: 'docker-publication',
    }).toString()
    result = await expectAccepted(await fetch(`${baseUrl}/slack/commands`, {
      method: 'POST',
      headers: slackHeaders(requiredEnv('SLACK_SECRET'), body, 'application/x-www-form-urlencoded'),
      body,
    }), connector)
  } else if (connector === 'telegram') {
    result = await expectAccepted(await fetch(`${baseUrl}/telegram/webhook`, {
      method: 'POST',
      headers: {
        'content-type': 'application/json',
        'x-telegram-bot-api-secret-token': requiredEnv('TELEGRAM_SECRET'),
      },
      body: JSON.stringify({
        update_id: 123,
        message: {
          message_id: 1,
          chat: { id: 789, type: 'private' },
          from: { id: 456, username: 'docker_drill' },
          text: 'docker-publication',
        },
      }),
    }), connector)
  } else if (connector === 'discord') {
    const body = JSON.stringify({
      type: 2,
      guild_id: 'guild-docker',
      member: { user: { id: 'user-docker', username: 'docker_drill' } },
      data: { name: 'arroba', options: [{ name: 'prompt', value: 'docker-publication' }] },
    })
    result = await expectAccepted(await fetch(`${baseUrl}/discord/interactions`, {
      method: 'POST',
      headers: discordHeaders(requiredEnv('DISCORD_PRIVATE_KEY_PEM'), body),
      body,
    }), connector)
  } else if (connector === 'whatsapp') {
    const body = JSON.stringify(whatsAppPayload())
    result = await expectAccepted(await fetch(`${baseUrl}/whatsapp/webhook`, {
      method: 'POST',
      headers: whatsAppHeaders(requiredEnv('WHATSAPP_SECRET'), body),
      body,
    }), connector)
  } else if (connector === 'signal') {
    result = await expectAccepted(await fetch(`${baseUrl}/signal/webhook`, {
      method: 'POST',
      headers: {
        'content-type': 'application/json',
        'x-signal-webhook-secret': requiredEnv('SIGNAL_SECRET'),
      },
      body: JSON.stringify({
        envelope: {
          sourceUuid: 'signal-source-uuid',
          sourceNumber: '+15551234567',
          dataMessage: { message: 'docker-publication' },
        },
      }),
    }), connector)
  } else {
    throw new Error(`unknown connector ${connector}`)
  }

  process.stdout.write(`${JSON.stringify({ connector, ok: true, result })}\n`)
}

main().catch((error) => {
  process.stderr.write(`${error.stack ?? error.message}\n`)
  process.exit(1)
})
