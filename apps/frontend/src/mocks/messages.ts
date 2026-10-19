import type { MessageDetail, MessageSummary } from '@/api/contracts'

const samples = [
  ['Build finished: api-gateway #1842', 'ci@northstar.dev', 'manu@northfoundry.dev', 'Northstar CI', 'The api-gateway checks passed on main.'],
  ['Re: Friday design review', 'elena@northfoundry.dev', 'manu@northfoundry.dev', 'Elena Rossi', 'I added the revised onboarding flow to the agenda.'],
  ['Your March usage report', 'reports@cloudharbor.io', 'dev@sender.test', 'CloudHarbor Reports', 'Your workspace used 18.4 GB this month.'],
  ['Invoice NF-2026-031', 'billing@papertrail.tools', 'accounts@northfoundry.dev', 'Papertrail Tools', 'The monthly invoice is ready for review.'],
  ['Staging deployment completed', 'deploy@northstar.dev', 'team@northfoundry.dev', 'Northstar Deploy', 'Version 2.8.1 is now running in staging.'],
  ['Welcome to Linear', 'notifications@linear.app', 'manu@northfoundry.dev', 'Linear', 'Your team workspace is ready.'],
  ['A new sign-in to your account', 'security@github.com', 'manu@northfoundry.dev', 'GitHub', 'A sign-in was detected from a new device.'],
  ['Re: Search indexing notes', 'luca@northfoundry.dev', 'elena@northfoundry.dev', 'Luca Bianchi', 'I have a small benchmark run to share tomorrow.'],
  ['Receipt for Figma Professional', 'team@figma.com', 'design@northfoundry.dev', 'Figma', 'Thanks for your payment. Your receipt is attached.'],
  ['Weekend maintenance window', 'ops@northfoundry.dev', 'engineering@northfoundry.dev', 'Northfoundry Operations', 'Maintenance is planned for Saturday at 22:00 UTC.'],
  ['New comment on “Inbox empty states”', 'notifications@figma.com', 'manu@northfoundry.dev', 'Figma', 'Marta left a comment on the empty state mockup.'],
  ['Password reset requested', 'no-reply@accounts.google.com', 'manu@northfoundry.dev', 'Google Accounts', 'A password reset link was requested for your account.'],
  ['Re: API pagination proposal', 'sara@northfoundry.dev', 'team@northfoundry.dev', 'Sara Conti', 'Keyset pagination looks good; please document the cursor.'],
  ['Your domain renews in 30 days', 'domains@cloudflare.com', 'infra@northfoundry.dev', 'Cloudflare', 'northfoundry.dev is set to renew automatically.'],
  ['Pull request review requested', 'notifications@github.com', 'manu@northfoundry.dev', 'GitHub', 'Elena requested your review on web#247.'],
  ['Daily digest: 3 updates', 'digest@linear.app', 'manu@northfoundry.dev', 'Linear', 'Three issues changed since your last visit.'],
  ['Welcome aboard, Giulia!', 'people@northfoundry.dev', 'all@northfoundry.dev', 'Northfoundry People', 'Giulia joins the platform team today.'],
  ['Backup completed successfully', 'alerts@backblaze.com', 'ops@northfoundry.dev', 'Backblaze', 'All 12 devices have a current backup.'],
  ['Re: Customer interview notes', 'marta@northfoundry.dev', 'elena@northfoundry.dev', 'Marta Greco', 'The main request was a keyboard shortcut for search.'],
  ['Certificate expires soon', 'monitor@northstar.dev', 'ops@northfoundry.dev', 'Northstar Monitor', 'The staging certificate expires in 14 days.'],
  ['You have been added to “Mail refresh”', 'notifications@github.com', 'manu@northfoundry.dev', 'GitHub', 'You now have access to the mail-refresh repository.'],
  ['Weekly product update', 'updates@postmarkapp.com', 'team@northfoundry.dev', 'Postmark', 'This week: improved deliverability analytics.'],
  ['Re: Localization strings', 'giulia@northfoundry.dev', 'manu@northfoundry.dev', 'Giulia Ferri', 'Italian copy is ready; I left two questions in the sheet.'],
  ['Your trial ends next week', 'hello@storybook.js.org', 'design@northfoundry.dev', 'Storybook', 'Choose a plan to keep your shared workspace active.'],
] as const

/**
 * Built-in, clearly synthetic inbox examples for local UI development.
 */
export const mockSummaries: MessageSummary[] = samples.map(([subject, sender, recipient], index) => ({
  id: `00000000-0000-4000-8000-${String(index + 1).padStart(12, '0')}`,
  sequence: samples.length - index,
  subject,
  from: [sender],
  to: [recipient],
  receivedAt: Date.UTC(2026, 9, 3 - Math.floor(index / 3), 15 - (index % 12), 24, 0),
  size: 1800 + index * 317,
  attachmentCount: 0,
  seen: false,
  preview: samples[index][4],
}))

const bodies = [
  'Build #1842 passed\n\nRepository: northstar/api-gateway\nBranch: main\nCommit: a4d83c1\n\n✓ TypeScript check      12s\n✓ Integration tests    48s\n✓ Container build      31s\n\nArtifacts are available for seven days. No action is needed.',
  'Hi Manu,\n\nI added the revised onboarding flow and the empty inbox state to Friday’s agenda. Could you bring the keyboard navigation proposal?\n\nSee you at 10:30,\nElena',
  'Your monthly workspace report\n\nStorage: 18.4 GB of 50 GB\nCaptured messages: 8,241\nActive projects: 6\n\nThe CSV attachment contains daily totals. Usage resets on the first day of the next month.',
  'Invoice NF-2026-031\n\nPapertrail Tools — Team plan\nBilling period: 1–31 March 2026\nSubtotal: €49.00\nVAT: €10.78\nTotal: €59.78\n\nPayment received. Keep the attached invoice for your records. Thank you for building with us.',
  'Deployment complete.\n\napi-gateway 2.8.1 is live in staging. Health checks passed across all three instances. Rollback target: 2.8.0.',
  'Welcome to your new workspace!\n\nYour invitation was accepted. Start by creating a project, inviting your team, and setting your first cycle. Your workspace URL is available in the account dashboard.',
  'New sign-in detected\n\nDevice: Chrome on macOS\nLocation: Milan, Italy (approximate)\nTime: 09:42 UTC\n\nIf this was you, no action is needed. Otherwise review active sessions in account settings and revoke any device you do not recognize.',
  'The benchmark completed in 312 ms for 50,000 records. Keyset pagination stayed stable while new messages arrived.\n\nI’ll add the query plan and allocation profile tomorrow.\n— Luca',
  'Payment receipt\n\nFigma Professional — 2 seats\nAmount paid: €30.00\nPayment method: card ending in 4242 (synthetic example)\nReceipt: FG-2026-0841\n\nYour subscription is active for another month. The attached HTML receipt is a local sample.',
  'Scheduled maintenance\n\nSaturday, 22:00–23:00 UTC\nAffected: staging database and preview environments\n\nDuring this window deployments may pause for up to ten minutes. Production traffic is unaffected. We will post completion in the engineering channel.',
  'Marta commented on Inbox empty states:\n\n“Could the empty state explain where to send the first email? A hostname and port would help a new developer.”\n\nReply in the design file when you have a draft.',
  'Verify your password reset\n\nYour verification code is 482 913.\n\nThis code expires in 15 minutes. Enter it in the account window that requested the reset. Never share this code. If you did not request a reset, ignore this message.',
  'I’m happy with the proposal. Please retain the exclusive before cursor and document that ordering is by capture sequence, not the sender’s Date header.\n\nOne edge case to test: reconnect after more than a full page of new mail.\n\nSara',
  'Domain renewal reminder\n\nnorthfoundry.dev renews in 30 days. Auto-renew is enabled. Please check that the billing contact and payment method are current. No payment is requested by this sample message.',
  'Review requested: web#247 — Improve empty inbox states\n\nAuthor: Elena Rossi\nChanges: 4 files, +86 / -32 lines\nChecks: passing\n\nThe pull request is ready for your feedback. Focus areas: accessibility labels and mobile message selection.',
  'Your daily digest\n\n• Mail refresh moved to In review\n• Keyboard shortcuts was assigned to Manu\n• HTML preview isolation received a new comment\n\nThere are no overdue issues in your current cycle.',
  'Please welcome Giulia to the platform team!\n\nGiulia will work on localization and developer experience. She starts today and will join our weekly demo on Thursday. Say hello when you get a moment.\n\nThe People team',
  'Backup completed successfully.\n\n12 devices backed up\n142 files changed\nTotal transferred: 843 MB\nIntegrity check: passed',
  'Hi Elena,\n\nThe interview notes are attached. The main requests were quick sender filtering, readable plain text, and a way to download the original message for debugging.\n\nNobody asked for a dashboard. They want to inspect the email and return to their editor.\n\nMarta',
  'Certificate alert\n\nHost: staging.northstar.test\nCertificate expires in 14 days\nRenewal job: last run failed\n\nCheck the DNS challenge credentials and rerun the renewal job. This is a synthetic development alert.',
  'You have been added to the Mail refresh repository.\n\nAccess level: Write\nInvited by: Elena Rossi\n\nYou can now clone the repository and review open pull requests.',
  'This week in product engineering\n\nBetter delivery diagnostics\nWe added a clearer timeline for each delivery attempt. Teams can now distinguish a temporary receiver failure from a configuration problem without searching through raw logs. The timeline keeps the original SMTP response alongside the normalized status.\n\nWorking with HTML email\nOur design team tested rendering across desktop and mobile clients. The most reliable layouts still use a narrow content column, inline styles, explicit image dimensions, and a useful plain-text alternative. Remember that remote images can be blocked by privacy settings. Your message should remain understandable without them.\n\nDeveloper workflow\nA good preview environment should be easy to reset and should make failed requests visible. This month we are simplifying our local setup so a new engineer can capture a verification email before connecting any external service.\n\nWhat is next\nWe are exploring saved searches and stronger accessibility for long message lists. These are research topics, not shipping announcements. If you have feedback on keyboard focus or screen reader output, we would love to see a small reproducible example.\n\nThanks for reading,\nThe product team',
  'Italian strings are ready.\n\nTwo questions remain:\n1. Should “Captured” be “Acquisita” or “Ricevuta”?\n2. Should dates follow the browser locale?\n\nI attached the draft strings as a CSV.\n\nGiulia',
  'Your trial ends next week.\n\nYour shared workspace remains available until 10 October. Review available plans in your account dashboard if you want to continue. Your local projects are unaffected.\n\nThank you for trying the service.',
];

/**
 * Return real synthetic attachment bytes together with their presentation metadata.
 */
export function mockAttachments(id: string) {
  const index = mockSummaries.findIndex((message) => message.id === id);
  const samples: Record<number, { filename: string; contentType: string; content: string }> = {
    2: { filename: 'daily-usage.csv', contentType: 'text/csv', content: 'date,messages,storage_mb\r\n2026-03-01,241,584\r\n2026-03-02,318,603\r\n' },
    3: { filename: 'invoice-NF-2026-031.txt', contentType: 'text/plain', content: bodies[3]! },
    8: { filename: 'receipt-FG-2026-0841.html', contentType: 'text/html', content: '<h1>Payment receipt</h1><p>Figma Professional — 2 seats</p><p>Total paid: €30.00</p>' },
    18: { filename: 'interview-notes.txt', contentType: 'text/plain', content: 'Interview notes\n\n1. Fast sender filtering\n2. Readable plain text\n3. Download original MIME\n' },
    22: { filename: 'it-strings.csv', contentType: 'text/csv', content: 'key,value\r\ninbox,Posta in arrivo\r\nattachments,Allegati\r\nsearch,Cerca nei messaggi caricati\r\n' },
  };
  const attachment = samples[index];
  return attachment ? [{ ...attachment, size: new TextEncoder().encode(attachment.content).length }] : [];
}

/*
 * Derive counts from the actual synthetic MIME parts used by raw-message downloads.
 */
for (const summary of mockSummaries) {
  summary.attachmentCount = mockAttachments(summary.id).length;
}

/**
 * Return deterministic message details matching each synthetic summary.
 */
export function mockDetail(id: string): MessageDetail | undefined {
  const index = mockSummaries.findIndex((message) => message.id === id);
  const summary = mockSummaries[index];
  if (!summary) return undefined;
  const text = bodies[index]!;
  const plainOnly = [7, 17, 19].includes(index);
  const receipt = index === 3 || index === 8;
  const html = plainOnly ? '' : `<html><head><style>body{margin:0;padding:28px;font:14px/1.65 system-ui;color:#27272a;background:#fff}article{max-width:580px;margin:auto}h1{font-size:22px;line-height:1.3}p{margin:0 0 18px}pre{padding:18px;background:#f4f4f5;white-space:pre-wrap}footer{border-top:1px solid #e4e4e7;padding-top:16px;color:#71717a;font-size:12px}</style></head><body><article><h1>${escapeHtml(receipt ? 'Payment confirmation' : summary.subject)}</h1>${index === 0 ? `<pre>${escapeHtml(text)}</pre>` : text.split('\n\n').map((paragraph) => `<p>${escapeHtml(paragraph).replaceAll('\n', '<br>')}</p>`).join('')}<footer>${escapeHtml(samples[index]![3])} · Local development sample</footer></article></body></html>`;
  return {
    id,
    subject: summary.subject,
    from: [...summary.from],
    to: [...summary.to],
    cc: [1, 12, 18].includes(index) ? ['team@northfoundry.dev', 'qa@northfoundry.dev'] : [],
    receivedAt: summary.receivedAt,
    text,
    html,
    htmlGenerated: false,
    attachments: mockAttachments(id).map(({ content, ...metadata }) => metadata),
    headers: {
      from: [...summary.from],
      to: [...summary.to],
      subject: [summary.subject],
      'message-id': [`<mock-${summary.sequence}@sandpost.local>`],
    },
  };
}

/**
 * Escape fixture text before embedding it inside the synthetic HTML document.
 */
function escapeHtml(value: string): string {
  return value.replace(/[&<>"']/g, (character) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[character]!);
}
