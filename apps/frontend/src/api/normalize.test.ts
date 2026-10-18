import { describe, expect, it } from 'vitest'
import { normalizeDetail, normalizeSummary, normalizeView, normalizeWebFolder, viewWire } from './normalize'

describe('API normalization', () => {
  it('round-trips custom View icons and normalizes omitted icons to the default', () => {
    const wire = { identifier: 'view', owner_identifier: null, name: 'QA', filter: 'true', icon: 'flask-conical' }
    const view = normalizeView(wire)
    expect(view.icon).toBe('flask-conical')
    expect(viewWire(view)).toEqual(wire)
    expect(viewWire({ ...view, icon: null }).icon).toBeNull()
    expect(normalizeView({ ...wire, icon: undefined }).icon).toBeNull()
  })

  it('retains custom manual folder icons and defaults older folder responses', () => {
    const wire = { identifier: 'folder', owner_identifier: 'owner', name: 'QA', exposed_accounts: [], created_at: 1, updated_at: 2 }
    expect(normalizeWebFolder({ ...wire, icon: 'bug' }).icon).toBe('bug')
    expect(normalizeWebFolder(wire).icon).toBeNull()
  })

  it('maps summary names, mailbox objects, and seconds to milliseconds', () => {
    expect(normalizeSummary({
      id: 'abc', seq: 9, subject: 'Hello', from: [{ address: 'a@b.test', domain: 'b.test' }],
      to: [{ address: 'c@d.test', domain: 'd.test' }], received_at: 123, size: 40, attachment_count: 2, seen: true,
    })).toEqual({
      id: 'abc', sequence: 9, subject: 'Hello', from: ['a@b.test'], to: ['c@d.test'],
      envelopeTo: ['c@d.test'], receivedAt: 123000, size: 40, attachmentCount: 2, seen: true,
      flags: { seen: true, answered: false, flagged: false, draft: false, deleted: false, keywords: [] },
    })
  })

  it('prefers normalized flags while retaining legacy Seen when flags are absent', () => {
    const base = { id: 'abc', seq: 1, subject: '', from: [], to: [], received_at: 1, size: 0, attachment_count: 0, seen: true }
    expect(normalizeSummary({ ...base, flags: { seen: false, answered: true, flagged: true, draft: false, deleted: false, keywords: ['work'] } }))
      .toMatchObject({ seen: false, flags: { seen: false, answered: true, flagged: true, deleted: false, keywords: ['work'] } })
    expect(normalizeSummary(base)).toMatchObject({ seen: true, flags: { seen: true, answered: false, flagged: false } })
  })

  it('flattens the detail envelope and maps optional attachment metadata', () => {
    expect(normalizeDetail({
      id: 'abc', facts: { subject: 'Hello', from: [{ address: 'a@b.test', domain: 'b.test' }], to: [], cc: [], received_at: 2, text: '', html: '', headers: { subject: ['Hello'] } },
      attachments: [{ filename: null, content_type: 'application/octet-stream', size: 4, content_hash: 'sample-hash' }],
    })).toMatchObject({ id: 'abc', subject: 'Hello', from: ['a@b.test'], receivedAt: 2000, attachments: [{ filename: null, contentType: 'application/octet-stream', size: 4 }] })
  })
})
