import { describe, expect, it } from 'vitest'
import { createMockClient } from './client'
import { mockAttachments, mockSummaries } from './messages'

describe('mock client', () => {
  it('requires first-run setup and owner login in its lifecycle state', async () => {
    const client = createMockClient()
    expect(await client.instanceStatus()).toEqual({ initialized: false, setupKeyRequired: true })
    await expect(client.login({ email: 'owner@example.test', password: 'password12345' })).rejects.toThrow('incorrect')
    await expect(client.authorizeSetup({ setupKey: 'wrong' })).rejects.toThrow('setup key')
    await expect(client.setup({
      bootstrapSecret: 'wrong', name: 'Owner', email: 'owner@example.test', password: 'password12345',
    })).rejects.toThrow('bootstrap secret')
    const { authorization } = await client.authorizeSetup({ setupKey: 'sandpost-test-secret' })
    expect(authorization).not.toBe('sandpost-test-secret')
    await client.setup({
      bootstrapSecret: authorization, name: 'Owner', email: 'owner@example.test', password: 'password12345',
    })
    expect(await client.instanceStatus()).toEqual({ initialized: true, setupKeyRequired: false })
    expect(await client.session()).toBeNull()
    expect(await client.login({ email: 'owner@example.test', password: 'password12345' })).toMatchObject({ role: 'owner' })
    expect(await client.session()).toMatchObject({ email: 'owner@example.test' })
    await client.logout()
    expect(await client.session()).toBeNull()
  })

  it('retains startup-key setup compatibility and enforces the backend password byte minimum', async () => {
    const client = createMockClient()
    await expect(client.setup({
      bootstrapSecret: 'sandpost-test-secret', name: 'Owner', email: 'owner@example.test', password: 'short',
    })).rejects.toThrow('password must contain 12 to 1024 bytes')
    await client.setup({
      bootstrapSecret: 'sandpost-test-secret', name: 'Owner', email: 'owner@example.test', password: 'password12345',
    })
    expect(await client.instanceStatus()).toEqual({ initialized: true, setupKeyRequired: false })
  })

  it('provides 24 detailed samples with keyset pagination and readable raw MIME', async () => {
    const client = createMockClient()
    const firstPage = await client.messages.list({ limit: 3 })
    expect(mockSummaries).toHaveLength(24)
    expect(firstPage.map(({ sequence }) => sequence)).toEqual([24, 23, 22])
    expect((await client.messages.list({ before: 23, limit: 2 })).map(({ sequence }) => sequence)).toEqual([22, 21])
    await expect(client.messages.list({ limit: 0 })).rejects.toThrow('limit must be between 1 and 100')
    const detail = await client.messages.get(firstPage[0].id)
    const raw = new TextDecoder().decode(await client.messages.getRaw(detail.id))
    expect(raw).toContain(`Subject: ${detail.subject}`)
    const encodedText = raw.match(/Content-Type: text\/plain; charset=utf-8\r\nContent-Transfer-Encoding: base64\r\n\r\n([A-Za-z0-9+/=\r\n]+)/)![1]!
    const decodedText = new TextDecoder().decode(Uint8Array.from(atob(encodedText.replace(/\s/g, '')), (character) => character.charCodeAt(0)))
    expect(decodedText).toBe(detail.text)
  })

  it('keeps inbox creation and message deletion per client, and returns exact attachment bytes', async () => {
    const client = createMockClient()
    const otherClient = createMockClient()
    const inboxes = await client.inboxes.list()
    expect(inboxes).toEqual([{
      id: '00000000-0000-0000-0000-000000000000', name: 'Default Inbox', address: null, system: true,
    }])
    const events: string[] = []
    const stop = await client.events.subscribe((event) => events.push(event.type))
    const inbox = await client.inboxes.create({ name: ' QA ', address: ' QA+Tag@Mail.Example ' })
    expect(inbox).toMatchObject({ name: 'QA', address: 'qa+tag@mail.example', system: false })
    expect((await client.inboxes.list()).map(({ name }) => name)).toEqual(['Default Inbox', 'QA'])
    expect(await otherClient.inboxes.list()).toHaveLength(1)
    expect(await client.messages.list({ inboxId: inbox.id })).toEqual([])
    expect(await otherClient.messages.list()).toHaveLength(24)
    for (const address of [
      'local', 'a@@example.test', 'a b@example.test', '"a"@example.test', 'a\\b@example.test',
      '.a@example.test', 'a..b@example.test', 'a.@example.test', 'a@-example.test',
      'a@example-.test', 'a@example..test', `${'a'.repeat(65)}@example.test`,
      `a@${'x'.repeat(64)}.test`, `${'a'.repeat(64)}@${'x'.repeat(186)}.test`,
      '\ta@example.test', 'a@example.test\r', 'a@exa\r\nmple.test', 'a@exämple.test',
    ]) {
      await expect(client.inboxes.create({ name: 'bad', address })).rejects.toThrow('Invalid inbox name or address')
    }

    const attachmentMail = (await client.messages.list()).find(({ attachmentCount }) => attachmentCount > 0)!
    const attachment = mockAttachments(attachmentMail.id)[0]!
    expect(await client.messages.getAttachment(attachmentMail.id, 0)).toEqual(new TextEncoder().encode(attachment.content))
    await expect(client.messages.getAttachment(attachmentMail.id, -1)).rejects.toThrow('Invalid attachment index')
    await client.messages.delete(attachmentMail.id)
    expect((await client.messages.list()).some(({ id }) => id === attachmentMail.id)).toBe(false)
    await expect(client.messages.get(attachmentMail.id)).rejects.toThrow('Mock message not found')
    await expect(client.messages.delete(attachmentMail.id)).rejects.toThrow('Mock message not found')
    expect(events).toEqual(['ready', 'resync', 'resync'])
    stop()
  })

  it('searches complete message content, headers, and recipients with keyset paging', async () => {
    const client = createMockClient()
    expect((await client.messages.search('312 ms')).map(({ subject }) => subject)).toContain('Re: Search indexing notes')
    expect((await client.messages.search('qa@northfoundry.dev')).length).toBeGreaterThan(0)
    expect((await client.messages.search('message-id', { limit: 1 })).map(({ sequence }) => sequence)).toEqual([24])
    expect((await client.messages.search('body-no-match'))).toEqual([])
    await expect(client.messages.search('a', { before: -1 })).rejects.toThrow('Invalid before cursor')
    await expect(client.messages.search(' ')).rejects.toThrow('search query must contain 1 to 256 characters')
    await expect(client.messages.search('x'.repeat(257))).rejects.toThrow('search query must contain 1 to 256 characters')
  })

  it('delivers resync when a mutation races the subscription promise', async () => {
    const client = createMockClient()
    const message = (await client.messages.list())[0]!
    const events: string[] = []
    const subscribing = client.events.subscribe((event) => events.push(event.type))
    await client.messages.delete(message.id)
    const stop = await subscribing
    expect(events).toEqual(['ready', 'resync'])
    stop()
  })

  it('includes long HTML, plain-only mail, Cc recipients, and downloadable attachment MIME parts', async () => {
    const client = createMockClient()
    const messages = await client.messages.list()
    const details = await Promise.all(messages.map((message) => client.messages.get(message.id)))
    expect(details.some((message) => message.text.length > 1000)).toBe(true)
    expect(details.some((message) => !message.html && message.text)).toBe(true)
    expect(details.some((message) => message.cc.length > 0)).toBe(true)
    const attachmentMail = details.find((message) => message.attachments.length > 0)!
    const raw = new TextDecoder().decode(await client.messages.getRaw(attachmentMail.id))
    expect(raw).toContain(`filename="${attachmentMail.attachments[0]!.filename}"`)
    expect(raw).toContain('Content-Type: multipart/alternative')
    const htmlMail = details.find((message) => message.html)!
    const htmlReport = await client.messages.htmlCheck(htmlMail.id)
    expect(htmlReport.htmlPresent).toBe(true)
    expect(htmlReport.summary.clients).toBe(htmlReport.clients.filter((client) => client.main).length)
    expect(htmlReport.summary.identical + htmlReport.summary.withDifferences + htmlReport.summary.withProblems + htmlReport.summary.untested).toBe(htmlReport.clients.filter((client) => client.main).length)
    expect(htmlReport.features.every((feature) => feature.support.every((result) => htmlReport.clients.some((client) => client.id === result.client)))).toBe(true)
    const plainMail = details.find((message) => !message.html)!
    expect(await client.messages.htmlCheck(plainMail.id)).toMatchObject({ htmlPresent: false, features: [] })
    await expect(client.messages.htmlCheck('missing')).rejects.toThrow('Mock message not found')
  })

  it('keeps IMAP accounts per owner, issues passwords once, and plans mailboxes from Smart Folders', async () => {
    const client = createMockClient()
    await client.setup({ bootstrapSecret: 'sandpost-test-secret', name: 'Owner', email: 'owner@example.test', password: 'password12345' })
    await expect(client.imap.list()).rejects.toThrow('authentication required')
    const owner = await client.login({ email: 'owner@example.test', password: 'password12345' })
    expect(await client.imap.server()).toMatchObject({ enabled: true, port: 1143, security: 'none' })
    const receipts = await client.views.save({ id: crypto.randomUUID(), ownerId: owner.id, name: 'Receipts', filter: 'subject contains "receipt"' })
    await client.views.save({ id: crypto.randomUUID(), ownerId: owner.id, name: 'a/b', filter: '' })

    const mirrored = await client.imap.create({ name: 'Apple Mail' })
    expect(mirrored.password).toHaveLength(32)
    expect(mirrored.account.username).toMatch(/^apple-mail-/)
    expect(mirrored.account.mailboxes.map(({ name }) => name)).toEqual(['INBOX', 'Trash', 'a-b', 'Receipts'])
    expect(JSON.stringify(await client.imap.list())).not.toContain(mirrored.password)
    expect(JSON.stringify(await client.imap.list())).not.toContain('mock-hash')

    const selected = await client.imap.create({
      name: 'Thunderbird', username: 'tb.local', mode: 'independent',
    })
    expect(selected.account.mailboxes.map(({ name, source }) => `${source}:${name}`)).toEqual(['inbox:INBOX', 'trash:Trash'])
    await expect(client.imap.create({ name: 'Copy', username: 'tb.local' })).rejects.toThrow('conflict')
    await expect(client.imap.create({ name: 'Bad', username: 'has space' })).rejects.toThrow('username must contain')
    await expect(client.imap.folders.create(mirrored.account.id, { name: 'Local', filter: '' }))
      .rejects.toThrow('Independent mode')

    const folder = await client.imap.folders.create(selected.account.id, { name: 'Welcome', filter: 'subject contains "welcome"' })
    expect(folder.includeInInbox).toBe(true)
    await expect(client.imap.folders.create(selected.account.id, { name: 'welcome', filter: '' })).rejects.toThrow('conflict')
    await expect(client.imap.folders.create(selected.account.id, { name: 'Broken', filter: 'subject contains "x' })).rejects.toThrow('unterminated string')
    await client.imap.folders.update(selected.account.id, folder.id, { name: 'Billing', filter: '', includeInInbox: false })
    expect((await client.imap.get(selected.account.id)).mailboxes.map(({ name }) => name)).toEqual(['INBOX', 'Trash', 'Billing'])

    await client.views.delete(receipts.id)
    const afterDeletion = await client.imap.get(selected.account.id)
    expect(afterDeletion.linkedViews).toEqual([])
    expect(afterDeletion.mailboxes.map(({ name }) => name)).toEqual(['INBOX', 'Trash', 'Billing'])

    const disabled = await client.imap.update(selected.account.id, { enabled: false, mode: 'mirror' })
    expect(disabled).toMatchObject({ enabled: false, mode: 'mirror', name: 'Thunderbird' })
    const regenerated = await client.imap.regeneratePassword(selected.account.id)
    expect(regenerated.password).not.toBe(selected.password)
    expect(await client.imap.restoreHidden(selected.account.id)).toBe(0)
    expect((await client.imap.get(selected.account.id)).folders).toEqual([])
    await client.imap.update(selected.account.id, { mode: 'independent' })
    expect((await client.imap.get(selected.account.id)).folders).toMatchObject([{ id: folder.id, name: 'Billing' }])
    await client.imap.folders.delete(selected.account.id, folder.id)
    await expect(client.imap.folders.delete(selected.account.id, folder.id)).rejects.toThrow('item not found')
    await client.imap.delete(selected.account.id)
    await expect(client.imap.get(selected.account.id)).rejects.toThrow('item not found')
    expect((await client.imap.list()).map(({ name }) => name)).toEqual(['Apple Mail'])
  })

  it('snapshots Independent folders while Mirror follows all Web folders and renames', async () => {
    const client = createMockClient();
    await client.setup({ bootstrapSecret: 'sandpost-test-secret', name: 'Owner', email: 'owner@example.test', password: 'password12345' });
    await client.login({ email: 'owner@example.test', password: 'password12345' });
    const original = await client.folders.create({ name: 'Original' });
    const mirror = (await client.imap.create({ name: 'Mirror' })).account;
    const independent = (await client.imap.create({ name: 'Independent', mode: 'independent' })).account;
    const second = (await client.imap.create({ name: 'Second', mode: 'independent' })).account;
    const local = await client.imap.folders.create(independent.id, { name: 'Local', filter: 'true' });
    expect((await client.imap.get(second.id)).folders.some((folder) => folder.id === local.id)).toBe(false);
    await client.folders.update(original.id, { name: 'Renamed' });
    await client.folders.create({ name: 'New' });
    expect((await client.imap.get(mirror.id)).mailboxes.map((mailbox) => mailbox.name)).toEqual(['INBOX', 'Trash', 'New', 'Renamed']);
    expect((await client.imap.get(independent.id)).mailboxes.map((mailbox) => mailbox.name)).toEqual(['INBOX', 'Trash', 'Local', 'Original']);
    await client.imap.update(independent.id, { mode: 'mirror' });
    await client.imap.update(independent.id, { mode: 'independent' });
    expect((await client.imap.get(independent.id)).folders).toContainEqual(local);
    await client.imap.delete(independent.id);
    expect((await client.imap.get(second.id)).folders).toHaveLength(1);
    expect(await client.folders.list()).toHaveLength(2);
  });

  it('shares Seen across primary lists, search, folders and Trash and emits only committed changes', async () => {
    const client = createMockClient();
    const other = createMockClient();
    await client.setup({ bootstrapSecret: 'sandpost-test-secret', name: 'Owner', email: 'owner@example.test', password: 'password12345' });
    await client.login({ email: 'owner@example.test', password: 'password12345' });
    const folder = await client.folders.create({ name: 'Seen Folder' });
    const message = (await client.messages.list())[0]!;
    const events: string[] = [];
    await client.events.subscribe((event) => events.push(event.type));
    expect(message.seen).toBe(false);
    expect(await client.messages.setSeen([message.id], true)).toBe(1);
    expect(await client.messages.setSeen([message.id], true)).toBe(0);
    expect((await client.messages.list())[0]?.seen).toBe(true);
    expect((await client.messages.search(message.subject))[0]?.seen).toBe(true);
    expect((await other.messages.list())[0]?.seen).toBe(false);
    await client.messages.move([message.id], folder.id);
    expect((await client.folders.listMessages(folder.id))[0]?.seen).toBe(true);
    await client.messages.trash([message.id]);
    expect((await client.trash.listMessages())[0]?.seen).toBe(true);
    expect(await client.messages.setSeen([message.id], false)).toBe(1);
    expect((await client.trash.listMessages())[0]?.seen).toBe(false);
    expect(events).toEqual(['ready', 'resync', 'resync', 'resync', 'resync']);
    await expect(client.messages.setSeen([message.id, message.id], true)).rejects.toThrow('Duplicate');
    await expect(client.messages.setSeen([message.id, crypto.randomUUID()], true)).rejects.toThrow('not found');
    expect((await client.trash.listMessages())[0]?.seen).toBe(false);
  });

  it('keeps manual folder placement, search visibility, and two-stage folder deletion consistent', async () => {
    const client = createMockClient()
    await client.setup({ bootstrapSecret: 'sandpost-test-secret', name: 'Owner', email: 'owner@example.test', password: 'password12345' })
    await client.login({ email: 'owner@example.test', password: 'password12345' })
    const account = await client.imap.create({ name: 'Thunderbird' })
    const folder = await client.folders.create({ name: 'To Review' })
    const message = (await client.messages.list({ limit: 1 }))[0]!

    expect((await client.imap.get(account.account.id)).mailboxes).toContainEqual(expect.objectContaining({ name: 'To Review' }))
    expect(await client.messages.move([message.id], folder.id)).toBe(1)
    expect(await client.folders.listMessages(folder.id)).toMatchObject([{ id: message.id }])
    expect((await client.messages.list()).some(({ id }) => id === message.id)).toBe(false)
    const smart = await client.views.save({ id: crypto.randomUUID(), ownerId: 'owner', name: 'Matching all', filter: `subject contains "${message.subject}"` })
    expect((await client.messages.list({ viewId: smart.id })).some(({ id }) => id === message.id)).toBe(false)
    expect((await client.messages.list({ query: 'true' })).some(({ id }) => id === message.id)).toBe(true)
    expect((await client.messages.search(message.subject)).some(({ id }) => id === message.id)).toBe(true)

    await client.messages.move([message.id], null)
    expect((await client.messages.list()).some(({ id }) => id === message.id)).toBe(true)
    await client.messages.move([message.id], folder.id)
    await client.folders.delete(folder.id)
    expect((await client.messages.list()).some(({ id }) => id === message.id)).toBe(false)
    expect(await client.trash.listMessages()).toMatchObject([{ id: message.id }])
    expect(await client.messages.get(message.id)).toMatchObject({ id: message.id })
  })

  it('copies primary memberships and mutates supported flags without changing Deleted', async () => {
    const client = createMockClient()
    await client.setup({ bootstrapSecret: 'sandpost-test-secret', name: 'Owner', email: 'owner@example.test', password: 'password12345' })
    await client.login({ email: 'owner@example.test', password: 'password12345' })
    const folder = await client.folders.create({ name: 'Receipts' })
    const message = (await client.messages.list({ limit: 1 }))[0]!
    expect(await client.messages.copy([message.id], folder.id)).toBe(1)
    expect((await client.messages.list())[0]?.id).toBe(message.id)
    expect((await client.folders.listMessages(folder.id))[0]?.id).toBe(message.id)
    await expect(client.messages.copy([message.id], folder.id)).rejects.toMatchObject({ code: 'conflict' })
    const patch = { seen: false, answered: false, flagged: true, draft: false, deleted: false, keywords: ['important', 'followup'] }
    expect(await client.messages.setFlags({ identifiers: [message.id], operation: 'add', flags: patch })).toBe(1)
    expect((await client.messages.list({ limit: 1 }))[0]?.flags).toMatchObject({ flagged: true, keywords: ['important', 'followup'], deleted: false })
    expect(await client.messages.setFlags({
      identifiers: [message.id], operation: 'remove',
      flags: { seen: false, answered: false, flagged: true, draft: false, deleted: false, keywords: ['followup'] },
    })).toBe(1)
    expect((await client.messages.list({ limit: 1 }))[0]?.flags).toMatchObject({ flagged: false, keywords: ['important'], deleted: false })
    await expect(client.messages.setFlags({
      identifiers: [message.id], operation: 'add',
      flags: { ...patch, deleted: true },
    })).rejects.toThrow('Deleted flag cannot be changed')
  })

  it('hides trashed mail everywhere but Trash, restores it by placement, and empties Trash permanently', async () => {
    const client = createMockClient()
    await client.setup({ bootstrapSecret: 'sandpost-test-secret', name: 'Owner', email: 'owner@example.test', password: 'password12345' })
    await client.login({ email: 'owner@example.test', password: 'password12345' })
    const folder = await client.folders.create({ name: 'Review' })
    const [first, second, third] = await client.messages.list({ limit: 3 })
    await client.messages.move([third!.id], folder.id)
    const events: string[] = []
    await client.events.subscribe((event) => events.push(event.type))

    expect(await client.messages.trash([first!.id, second!.id, third!.id])).toBe(3)
    expect(events.at(-1)).toBe('resync')

    /**
     * Check whether a message identifier appears in the supplied visible page.
     */
    const visible = (messages: { id: string }[], id: string) => messages.some((message) => message.id === id)
    expect(visible(await client.messages.list(), first!.id)).toBe(false)
    expect(visible(await client.folders.listMessages(folder.id), third!.id)).toBe(false)
    expect(visible(await client.messages.list({ query: 'true' }), first!.id)).toBe(false)
    expect(visible(await client.messages.search(first!.subject), first!.id)).toBe(false)
    expect((await client.trash.listMessages()).map(({ id }) => id)).toEqual([first!.id, second!.id, third!.id])
    expect((await client.trash.listMessages({ before: first!.sequence, limit: 1 })).map(({ id }) => id)).toEqual([second!.id])
    expect(await client.messages.get(first!.id)).toMatchObject({ id: first!.id })
    await expect(client.messages.trash([])).rejects.toThrow('1 to 256')
    await expect(client.messages.trash([first!.id, first!.id])).rejects.toThrow('Duplicate')

    expect(await client.messages.move([first!.id], null)).toBe(1)
    expect(visible(await client.messages.list(), first!.id)).toBe(true)
    await client.messages.move([third!.id], folder.id)
    expect(visible(await client.folders.listMessages(folder.id), third!.id)).toBe(true)
    await client.messages.trash([third!.id])
    await client.messages.delete(third!.id)
    expect((await client.trash.listMessages()).map(({ id }) => id)).toEqual([second!.id])

    expect(await client.trash.empty()).toBe(1)
    expect(await client.trash.listMessages()).toEqual([])
    await expect(client.messages.get(second!.id)).rejects.toThrow('not found')
    expect(visible(await client.messages.list(), second!.id)).toBe(false)
    expect(await client.trash.empty()).toBe(0)
  })
})
