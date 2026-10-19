import * as VueRuntime from 'vue';
import { computed, createRenderer, defineComponent, h, nextTick, reactive, ref, ssrContextKey } from 'vue';
import { compileScript, compileTemplate, parse } from '@vue/compiler-sfc';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { MessageDetail, SandpostClient, SandpostEvent, SessionUser } from '@/api/contracts';
import type { MailFolder, Workspace } from '@/features/shell/workspace';
import { i18n } from '@/i18n';

const harness = vi.hoisted(() => ({
  route: null as unknown as { name: string; params: Record<string, string | undefined>; query: Record<string, string | undefined> },
  client: {} as SandpostClient,
  workspace: {} as Workspace,
  router: { push: vi.fn(), replace: vi.fn(async () => { }) },
  lists: [] as {
    props: Record<string, any>;

    /**
     * Emit a captured component event through the test harness.
     */
    emit: (...args: any[]) => void
  }[],
  details: [] as {
    props: Record<string, any>;

    /**
     * Emit a captured component event through the test harness.
     */
    emit: (...args: any[]) => void
  }[],
  selectionPanels: [] as {
    props: Record<string, any>;

    /**
     * Emit a captured component event through the test harness.
     */
    emit: (...args: any[]) => void
  }[],
}));

vi.mock('vue-router', () => ({
  /**
   * Expose the route double configured by the component test.
   */
  useRoute: () => harness.route,

  /**
   * Expose the router double configured by the component test.
   */
  useRouter: () => harness.router,
}));

vi.mock('@/runtime/client', () => ({
  /**
   * Expose the client double configured by the component test.
   */
  useSandpostClient: () => harness.client
}));
vi.mock('@/features/shell/workspace', async (importOriginal) => ({
  ...await importOriginal<typeof import('@/features/shell/workspace')>(),

  /**
   * Expose the workspace double configured by the component test.
   */
  useWorkspace: () => harness.workspace,
}));

/*
 * Visual primitives render as plain elements so the page template can be inspected directly.
 */
const { passthrough } = vi.hoisted(() => ({
  /**
   * Create a visual component stand-in that forwards attributes and slots to the requested host tag.
   */
  passthrough: (tag: string) => defineComponent({
    /**
     * Create the component stand-in render function used by the custom Vue test renderer.
     */
    setup: (_, { attrs, slots }) => () => h(tag, attrs, slots.default?.())
  }),
}));
vi.mock('@/components/ui/alert', () => ({ Alert: passthrough('div'), AlertDescription: passthrough('div') }));
vi.mock('@/components/ui/button', () => ({ Button: passthrough('button') }));
vi.mock('@/components/ui/dialog', () => ({ DialogFooter: passthrough('div') }));
vi.mock('@/components/ui/spinner', () => ({ Spinner: passthrough('span') }));
vi.mock('@/components/BusyDialog.vue', () => ({
  default: defineComponent({
    props: ['open', 'title', 'description', 'busy'],

    /**
     * Create the component stand-in render function used by the custom Vue test renderer.
     */
    setup: (props, { slots }) => () => props.open ? h('dialog', { title: props.title, description: props.description }, slots.default?.()) : null,
  }),
}));
vi.mock('@/features/mail/MessageSelectionPanel.vue', () => ({
  default: defineComponent({
    props: ['count', 'deleting', 'folders', 'trash', 'canMutate', 'checkingLinks', 'linkCheckNotice'],
    emits: ['delete', 'move', 'copy', 'set-flags', 'clear', 'check-links'],

    /**
     * Create the component stand-in render function used by the custom Vue test renderer.
     */
    setup: (props, { emit }) => {
      harness.selectionPanels.push({ props, emit });
      return () => h('section');
    },
  }),
}));

vi.mock('@/features/mail/MessageList.vue', () => ({
  default: defineComponent({
    props: ['messages', 'folder', 'search', 'loading', 'hasOlder', 'error', 'selection', 'showEmptyTrash', 'emptyTrashDisabled', 'readMessageIds', 'seenBusy', 'canMutate', 'checkingLinks', 'linkCheckNotice'],
    emits: ['select', 'clearSearch', 'refresh', 'older', 'filter', 'update:selection', 'delete', 'move', 'emptyTrash', 'set-seen', 'check-links'],

    /**
     * Create the component stand-in render function used by the custom Vue test renderer.
     */
    setup: (props, { emit }) => {
      harness.lists.push({ props, emit });
      return () => h('div');
    },
  }),
}));

vi.mock('@/features/mail/MessageDetail.vue', () => ({
  default: defineComponent({
    props: ['message', 'loading', 'error', 'mobile', 'downloadingAttachment', 'deleting', 'folders', 'trash', 'seen', 'seenBusy', 'flags', 'canMutate'],
    emits: ['back', 'retry', 'download', 'attachment', 'delete', 'move', 'copy', 'set-seen', 'set-flags'],

    /**
     * Create the component stand-in render function used by the custom Vue test renderer.
     */
    setup: (props, { emit }) => {
      harness.details.push({ props, emit });
      return () => h('div');
    },
  }),
}));

import MailPage from './MailPage.vue';
import pageSource from './MailPage.vue?raw';

const pageDescriptor = parse(pageSource, { filename: 'MailPage.vue' }).descriptor;
const pageBindings = compileScript(pageDescriptor, { id: 'mail-page-regression' }).bindings;
const pageTemplate = compileTemplate({
  id: 'mail-page-regression',
  filename: 'MailPage.vue',
  source: pageDescriptor.template!.content,
  compilerOptions: { bindingMetadata: pageBindings },
});
const clientRender = new Function('VueRuntime', pageTemplate.code
  .replace(/^import \{([\s\S]*?)\} from "vue"\s*;?\s*/m, (_statement, bindings: string) =>
    `const {${bindings.replace(/\s+as\s+/g, ': ')}} = VueRuntime;`)
  .replace('export function render', 'return function render'))(VueRuntime) as (this: unknown) => unknown;
(MailPage as typeof MailPage & { render: typeof clientRender }).render = clientRender;

interface HostNode {
  type: string;
  props: Record<string, unknown>;
  children: HostNode[];
  parent: HostNode | null;
  text?: string;
}

const renderer = createRenderer<HostNode, HostNode>({
  /**
   * Create an element host node with the state required by the custom Vue test renderer.
   */
  createElement: (type) => ({ type, props: {}, children: [], parent: null }),

  /**
   * Create a text host node for the custom Vue test renderer.
   */
  createText: (text) => ({ type: '#text', text, props: {}, children: [], parent: null }),

  /**
   * Create a comment host node for the custom Vue test renderer.
   */
  createComment: (text) => ({ type: '#comment', text, props: {}, children: [], parent: null }),

  /**
   * Update the text stored on a host node by the custom Vue test renderer.
   */
  setText: (node, text) => { node.text = text; },

  /**
   * Replace an element host node text and clear its previous child nodes.
   */
  setElementText: (node, text) => { node.text = text; node.children = []; },

  /**
   * Store the updated component attribute on its host-node test double.
   */
  patchProp: (node, key, _previous, next) => { node.props[key] = next; },

  /**
   * Attach a host node to its test parent at the requested anchor and record its parent.
   */
  insert: (node, parent, anchor) => {
    if (node.parent) node.parent.children.splice(node.parent.children.indexOf(node), 1);
    const index = anchor ? parent.children.indexOf(anchor) : -1;
    if (index < 0) parent.children.push(node);
    else parent.children.splice(index, 0, node);
    node.parent = parent;
  },

  /**
   * Detach the host node from its recorded test parent.
   */
  remove: (node) => {
    if (node.parent) node.parent.children.splice(node.parent.children.indexOf(node), 1);
    node.parent = null;
  },

  /**
   * Return the parent recorded for this host node by the custom Vue test renderer.
   */
  parentNode: (node) => node.parent,

  /**
   * Return the next host node in the test parent, or null at the end of its children.
   */
  nextSibling: (node) => {
    if (!node.parent) return null;
    return node.parent.children[node.parent.children.indexOf(node) + 1] ?? null;
  },

  /**
   * Insert a static-content host node and return its start and end nodes to Vue.
   */
  insertStaticContent(content, parent, anchor) {
    const node: HostNode = { type: '#static', text: content, props: {}, children: [], parent: null };
    const index = anchor ? parent.children.indexOf(anchor) : -1;
    if (index < 0) parent.children.push(node);
    else parent.children.splice(index, 0, node);
    node.parent = parent;
    return [node, node];
  },
});


const DEFAULT_ID = '00000000-0000-0000-0000-000000000000';
const message: MessageDetail = {
  id: 'message-1', subject: 'Receipt', from: ['sender@example.test'], to: ['qa@sandpost.local'], cc: [], receivedAt: 1_760_000_000_000,
  text: 'Receipt body', html: '', attachments: [{ filename: 'receipt.txt', contentType: 'text/plain', size: 8 }], headers: {}, htmlGenerated: false,
};
const folders: MailFolder[] = [
  { id: DEFAULT_ID, kind: 'inbox', name: 'Inbox', address: null, view: null },
  { id: 'trash', kind: 'trash', name: 'Trash', address: null, view: null },
  { id: 'address-1', kind: 'address', name: 'QA', address: 'qa@sandpost.local', view: null },
  { id: 'smart-1', kind: 'smart', name: 'Receipts', address: null, view: { id: 'smart-1', ownerId: 'owner', name: 'Receipts', filter: 'subject contains "Receipt"' } },
  { id: 'manual-1', kind: 'folder', name: 'To Review', address: null, view: null },
];

/**
 * Flushes pending client promises and Vue prop updates after a captured UI event.
 */
async function flushUi(): Promise<void> {
  for (let index = 0; index < 3; index += 1) {
    await Promise.resolve();
    await nextTick();
  }
}

/**
 * Installs the browser globals used by page lifecycle and file-download handlers.
 */
function installBrowserStubs(): void {
  const anchor = { href: '', download: '', click: vi.fn(), remove: vi.fn() };
  vi.stubGlobal('document', {
    documentElement: { dataset: {} },
    createElement: vi.fn(() => anchor),
    body: { append: vi.fn() },
  });
  vi.stubGlobal('localStorage', { getItem: vi.fn(() => null), setItem: vi.fn() });
  vi.stubGlobal('window', { addEventListener: vi.fn(), removeEventListener: vi.fn() });
  vi.stubGlobal('URL', { createObjectURL: vi.fn(() => 'blob:test'), revokeObjectURL: vi.fn() });
}

/**
 * Builds a workspace double whose folder list and event stream the test controls.
 */
function createWorkspaceDouble(role: SessionUser['role'] = 'member'): {
  workspace: Workspace;

  /**
   * Forward a mail event to the listeners registered on this workspace double.
   */
  emitEvent: (event: SandpostEvent) => void
} {
  const listeners = new Set<(event: SandpostEvent) => void>();
  const folderList = ref(folders);
  const viewedMessageIds = ref(new Set<string>());
  const workspace = {
    user: ref<SessionUser>({ id: 'user', name: 'User', email: 'user@example.test', role }),
    folders: computed(() => folderList.value),
    foldersLoaded: ref(true),
    viewedMessageIds,
    setSeen: vi.fn(async (identifiers: string[], seen: boolean) => {
      await harness.client.messages.setSeen(identifiers, seen);
      const updated = new Set(viewedMessageIds.value);
      for (const id of identifiers) seen ? updated.add(id) : updated.delete(id);
      viewedMessageIds.value = updated;
    }),
    refreshInboxUnread: vi.fn(),

    /**
     * Resolve a test folder identifier using the default Inbox for an empty selection.
     */
    findFolder: (id: string | null | undefined) => folderList.value.find((folder) => folder.id === (id || DEFAULT_ID)) ?? null,

    /**
     * Register a test mail listener and return the function that removes it.
     */
    onMailEvent: (listener: (event: SandpostEvent) => void) => { listeners.add(listener); return () => listeners.delete(listener); },
  } as unknown as Workspace;
  return {
    workspace,

    /**
     * Forward a mail event to the listeners registered on this workspace double.
     */
    emitEvent: (event) => { for (const listener of listeners) listener(event); }
  };
}

const pageRoot: HostNode = { type: 'root', props: {}, children: [], parent: null };

/**
 * Mounts the page with the custom Vue renderer and returns its app for cleanup.
 */
function mountPage() {
  i18n.global.locale.value = 'en';
  const app = renderer.createApp(MailPage);
  app.use(i18n);
  app.provide(ssrContextKey, { modules: new Set() });
  pageRoot.children = [];
  app.mount(pageRoot);
  return app;
}

/**
 * Concatenates the rendered text below a host node, ignoring the comments Vue leaves for inactive branches.
 */
function textOf(node: HostNode): string {
  if (node.type === '#comment') return '';
  return (node.text ?? '') + node.children.map(textOf).join('');
}

/**
 * Finds the rendered button whose trimmed text equals `label`.
 */
function findButton(node: HostNode, label: string): HostNode | undefined {
  if (node.type === 'button' && textOf(node).trim() === label) return node;
  for (const child of node.children) {
    const found = findButton(child, label);
    if (found) return found;
  }
  return undefined;
}

/**
 * Returns the open confirmation dialog, if any.
 */
function dialog(node: HostNode = pageRoot): HostNode | undefined {
  if (node.type === 'dialog') return node;
  for (const child of node.children) {
    const found = dialog(child);
    if (found) return found;
  }
  return undefined;
}

/**
 * Builds list summaries for the given message ids, newest first.
 */
function summaries(...ids: string[]) {
  return ids.map((id, index) => ({ id, sequence: ids.length - index, subject: `Subject ${id}`, from: [], to: [], receivedAt: 0, size: 1, attachmentCount: 0, seen: false }));
}

describe('MailPage', () => {
  let emitEvent: (event: SandpostEvent) => void;

  beforeEach(() => {
    vi.useFakeTimers();
    installBrowserStubs();
    harness.route = reactive({ name: 'mail', params: {}, query: {} });
    harness.lists = [];
    harness.details = [];
    harness.selectionPanels = [];
    vi.clearAllMocks();
    ({ workspace: harness.workspace, emitEvent } = createWorkspaceDouble());
    harness.client = {
      messages: {
        list: vi.fn(async () => []), search: vi.fn(async () => []), get: vi.fn(async () => message),
        getRaw: vi.fn(async () => new Uint8Array([1])), delete: vi.fn(async () => undefined), move: vi.fn(async () => 1),
        setSeen: vi.fn(async (ids: string[]) => ids.length),
        copy: vi.fn(async (ids: string[]) => ids.length), setFlags: vi.fn(async () => 1),
        trash: vi.fn(async (ids: string[]) => ids.length), getAttachment: vi.fn(async () => new Uint8Array([1])),
        checkLinks: vi.fn(async (ids: string[]) => ({ started: ids.length, alreadyRunning: 0, skipped: 0 })),
      },
      folders: { listMessages: vi.fn(async () => []) },
      trash: { listMessages: vi.fn(async () => []), empty: vi.fn(async () => 0) },
    } as unknown as SandpostClient;
  });

  afterEach(() => {
    vi.runOnlyPendingTimers();
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it('reads the inbox, address inboxes, and smart folders through their own list filters', async () => {
    const app = mountPage();
    await flushUi();
    expect(harness.client.messages.list).toHaveBeenLastCalledWith({ before: undefined, limit: 50 });

    harness.route.query = { folder: 'address-1' };
    await flushUi();
    expect(harness.client.messages.list).toHaveBeenLastCalledWith({ inboxId: 'address-1', before: undefined, limit: 50 });
    expect(harness.lists[0]!.props.folder).toMatchObject({ kind: 'address', address: 'qa@sandpost.local' });

    harness.route.query = { folder: 'smart-1' };
    await flushUi();
    expect(harness.client.messages.list).toHaveBeenLastCalledWith({ viewId: 'smart-1', before: undefined, limit: 50 });
    app.unmount();
  });

  it('searches all readable mail with the canonical form of shortcut syntax', async () => {
    harness.route.query = { folder: 'smart-1', q: 'from:dev@example.test' };
    const app = mountPage();
    await flushUi();
    expect(harness.client.messages.list).toHaveBeenLastCalledWith({
      query: '(from.address contains "dev@example.test" or envelope.from.address contains "dev@example.test")', before: undefined, limit: 50,
    });

    harness.lists[0]!.emit('clearSearch');
    expect(harness.router.replace).toHaveBeenCalledWith({ name: 'mail', params: {}, query: { folder: 'smart-1' } });
    app.unmount();
  });

  it('lists manual folders through their endpoint and moves selected messages', async () => {
    harness.route.query = { folder: 'manual-1' };
    const app = mountPage();
    await flushUi();
    expect(harness.client.folders.listMessages).toHaveBeenLastCalledWith('manual-1', { before: undefined, limit: 50 });
    expect(harness.lists[0]!.props.folder).toMatchObject({ kind: 'folder', name: 'To Review' });
    harness.lists[0]!.emit('move', ['message-a', 'message-b'], 'manual-1');
    await flushUi();
    expect(harness.client.messages.move).toHaveBeenCalledWith(['message-a', 'message-b'], 'manual-1');
    expect(harness.client.folders.listMessages).toHaveBeenCalledTimes(2);
    app.unmount();
  });

  it('marks an opened message read, allows unread, and accepts shared Seen changes without reopening it', async () => {
    let rows = summaries('message-1');
    vi.mocked(harness.client.messages.list).mockImplementation(async () => rows);
    vi.mocked(harness.client.messages.setSeen).mockImplementation(async (identifiers, seen) => {
      rows = rows.map((row) => identifiers.includes(row.id) ? { ...row, seen } : row);
      return identifiers.length;
    });
    const app = mountPage();
    await flushUi();
    harness.route.name = 'mail-message';
    harness.route.params.id = 'message-1';
    await flushUi();
    expect(harness.client.messages.setSeen).toHaveBeenCalledWith(['message-1'], true);
    expect(harness.details[0]!.props.seen).toBe(true);
    harness.details[0]!.emit('set-seen', false);
    await flushUi();
    expect(harness.client.messages.setSeen).toHaveBeenLastCalledWith(['message-1'], false);
    expect(harness.details[0]!.props.seen).toBe(false);
    expect(harness.lists[0]!.props.readMessageIds.has('message-1')).toBe(false);
    rows = rows.map((row) => ({ ...row, seen: true }));
    emitEvent({ type: 'resync' });
    await flushUi();
    expect(harness.details[0]!.props.seen).toBe(true);
    expect(harness.client.messages.setSeen).toHaveBeenCalledTimes(2);
    app.unmount();
  });

  it('keeps viewer reads from changing Seen and hides mutation actions', async () => {
    (harness.workspace.user as any).value.role = 'viewer';
    harness.route.name = 'mail-message';
    harness.route.params.id = 'message-1';
    const app = mountPage(); await flushUi();
    expect(harness.client.messages.setSeen).not.toHaveBeenCalled();
    expect(harness.lists[0]!.props.canMutate).toBe(false);
    expect(harness.details[0]!.props.canMutate).toBe(false);
    harness.details[0]!.emit('set-seen', false);
    await flushUi();
    expect(harness.client.messages.setSeen).not.toHaveBeenCalled();
    app.unmount();
  });

  it('keeps read and unread actions accurate for a deep-linked message outside the loaded list', async () => {
    harness.route.name = 'mail-message'; harness.route.params.id = 'message-1';
    const app = mountPage(); await flushUi();
    expect(harness.client.messages.setSeen).toHaveBeenCalledWith(['message-1'], true);
    expect(harness.details[0]!.props.seen).toBe(true);
    harness.details[0]!.emit('set-seen', false); await flushUi();
    expect(harness.details[0]!.props.seen).toBe(false);
    emitEvent({ type: 'resync' }); await flushUi();
    expect(harness.details[0]!.props.seen).toBe(false);
    expect(harness.client.messages.setSeen).toHaveBeenCalledTimes(2);
    app.unmount();
  });

  it('batches selected read changes and leaves failed changes unread', async () => {
    vi.mocked(harness.client.messages.list).mockResolvedValue(summaries('message-1'));
    const app = mountPage();
    await flushUi();
    const identifiers = Array.from({ length: 300 }, (_, index) => `message-${index + 1}`);
    harness.lists[0]!.emit('set-seen', identifiers, true);
    await flushUi();
    expect(harness.client.messages.setSeen).toHaveBeenNthCalledWith(1, identifiers.slice(0, 256), true);
    expect(harness.client.messages.setSeen).toHaveBeenNthCalledWith(2, identifiers.slice(256), true);
    vi.mocked(harness.client.messages.setSeen).mockRejectedValueOnce(new Error('Seen update failed'));
    harness.lists[0]!.emit('set-seen', ['message-1'], false);
    await flushUi();
    expect(harness.lists[0]!.props.readMessageIds.has('message-1')).toBe(true);
    expect(harness.lists[0]!.props.error).toBe('Seen update failed');
    app.unmount();
  });

  it('falls back to the inbox when the folder in the URL no longer exists', async () => {
    harness.route.query = { folder: 'deleted-folder' };
    const app = mountPage();
    await flushUi();
    expect(harness.router.replace).toHaveBeenCalledWith({ name: 'mail', query: {} });
    app.unmount();
  });

  it('keeps the folder and search when opening a message and reloads on mail events', async () => {
    harness.route.query = { folder: 'address-1' };
    vi.mocked(harness.client.messages.list).mockResolvedValue([
      { id: 'message-1', sequence: 4, subject: 'Receipt', from: [], to: [], receivedAt: 0, size: 1, attachmentCount: 0, seen: false },
    ]);
    const app = mountPage();
    await flushUi();
    harness.lists[0]!.emit('select', 'message-1');
    expect(harness.router.push).toHaveBeenCalledWith({ name: 'mail-message', params: { id: 'message-1' }, query: { folder: 'address-1' } });

    const calls = vi.mocked(harness.client.messages.list).mock.calls.length;
    emitEvent({ type: 'message', id: 'new', sequence: 5 });
    await flushUi();
    expect(harness.client.messages.list).toHaveBeenCalledTimes(calls + 1);
    emitEvent({ type: 'disconnected' });
    await flushUi();
    expect(harness.client.messages.list).toHaveBeenCalledTimes(calls + 1);
    app.unmount();
  });

  it.each([
    {
      label: 'attachment',

      /**
       * Select the client operation that this download scenario forces to fail.
       */
      fail: (client: SandpostClient) => client.messages.getAttachment, emit: 'attachment', args: [0],

      /**
       * Select the client operation whose calls this download scenario verifies.
       */
      calls: (client: SandpostClient) => client.messages.getAttachment
    },
    {
      label: 'raw message',

      /**
       * Select the client operation that this download scenario forces to fail.
       */
      fail: (client: SandpostClient) => client.messages.getRaw, emit: 'download', args: [],

      /**
       * Select the client operation whose calls this download scenario verifies.
       */
      calls: (client: SandpostClient) => client.messages.getRaw
    },
  ])('retries the failed $label download instead of reloading message details', async ({ fail, emit, args, calls }) => {
    harness.route = reactive({ name: 'mail-message', params: { id: message.id }, query: {} });
    const app = mountPage();
    await flushUi();
    vi.mocked(fail(harness.client)).mockRejectedValueOnce(new Error('download failed'));

    harness.details[0]!.emit(emit, ...args);
    await flushUi();
    harness.details[0]!.emit('retry');
    await flushUi();

    expect(calls(harness.client)).toHaveBeenCalledTimes(2);
    expect(harness.client.messages.get).toHaveBeenCalledTimes(1);
    app.unmount();
  });

  it('shows the selection panel for several messages and opens a selection narrowed to one', async () => {
    vi.mocked(harness.client.messages.list).mockResolvedValue(summaries('a', 'b', 'c'));
    const app = mountPage();
    await flushUi();

    harness.lists[0]!.emit('update:selection', new Set(['c', 'a']));
    await flushUi();
    expect(harness.selectionPanels.at(-1)!.props.count).toBe(2);

    harness.lists[0]!.emit('update:selection', new Set(['b']));
    await flushUi();
    expect(harness.lists[0]!.props.selection.size).toBe(0);
    expect(harness.router.push).toHaveBeenCalledWith({ name: 'mail-message', params: { id: 'b' }, query: {} });
    app.unmount();
  });

  it('copies selected messages and applies primary flags through the client API', async () => {
    vi.mocked(harness.client.messages.list).mockResolvedValue(summaries('message-a', 'message-b'));
    const app = mountPage(); await flushUi();
    harness.lists[0]!.emit('update:selection', new Set(['message-a', 'message-b'])); await flushUi();
    harness.selectionPanels.at(-1)!.emit('copy', 'folder-id'); await flushUi();
    expect(harness.client.messages.copy).toHaveBeenCalledWith(['message-a', 'message-b'], 'folder-id');
    harness.lists[0]!.emit('update:selection', new Set(['message-a', 'message-b'])); await flushUi();
    const flags = { seen: false, answered: false, flagged: true, draft: false, deleted: false, keywords: [] };
    harness.selectionPanels.at(-1)!.emit('set-flags', 'add', flags); await flushUi();
    expect(harness.client.messages.setFlags).toHaveBeenCalledWith({ identifiers: ['message-a', 'message-b'], operation: 'add', flags });
    app.unmount();
  });

  it('allows every role to start bulk link checks while viewers remain read-only', async () => {
    vi.mocked(harness.client.messages.list).mockResolvedValue(summaries('message-a', 'message-b'));
    for (const role of ['owner', 'member', 'viewer'] as const) {
      harness.workspace = createWorkspaceDouble(role).workspace;
      const app = mountPage(); await flushUi();
      harness.lists.at(-1)!.emit('update:selection', new Set(['message-a', 'message-b'])); await flushUi();
      const selectionPanel = harness.selectionPanels.at(-1)!;
      expect(selectionPanel.props).toMatchObject({ canMutate: role !== 'viewer', count: 2 });
      selectionPanel.emit('check-links'); await flushUi();
      expect(harness.client.messages.checkLinks).toHaveBeenNthCalledWith(
        role === 'owner' ? 1 : role === 'member' ? 2 : 3,
        ['message-a', 'message-b'],
      );
      expect(harness.selectionPanels.at(-1)!.props.linkCheckNotice).toBe('Checking links in 2 messages. Results appear when you open them.');
      expect(harness.lists.at(-1)!.props.selection.size).toBe(2);
      app.unmount();
    }
  });

  it('keeps inspector flags in sync after opening or marking a message unread', async () => {
    const summary = { ...summaries('message-1')[0]!, seen: false, flags: { seen: false, answered: true, flagged: true, draft: false, deleted: false, keywords: ['Work'] } };
    harness.route = reactive({ name: 'mail-message', params: { id: 'message-1' }, query: {} });
    vi.mocked(harness.client.messages.list).mockResolvedValue([summary]);
    const app = mountPage(); await flushUi();
    expect(harness.details.at(-1)!.props.flags).toEqual({ ...summary.flags, seen: true });
    harness.details.at(-1)!.emit('set-seen', false); await flushUi();
    expect(harness.details.at(-1)!.props.flags).toEqual({ ...summary.flags, seen: false });
    app.unmount();
  });

  it('keeps a rejected COPY selection available for retry', async () => {
    vi.mocked(harness.client.messages.list).mockResolvedValue(summaries('message-a', 'message-b'));
    vi.mocked(harness.client.messages.copy).mockRejectedValueOnce(new Error('Destination already contains a message'));
    const app = mountPage(); await flushUi();
    harness.lists[0]!.emit('update:selection', new Set(['message-a', 'message-b'])); await flushUi();
    harness.selectionPanels.at(-1)!.emit('copy', 'folder-id'); await flushUi();
    expect(harness.lists[0]!.props.selection).toEqual(new Set(['message-a', 'message-b']));
    expect(harness.lists[0]!.props.error).toBe('Destination already contains a message');
    app.unmount();
  });

  it('permanently deletes confirmed messages inside Trash and closes the open one', async () => {
    harness.route = reactive({ name: 'mail-message', params: { id: 'b' }, query: { folder: 'trash' } });
    vi.mocked(harness.client.trash.listMessages).mockResolvedValueOnce(summaries('a', 'b', 'c', 'd')).mockResolvedValue(summaries('a'));
    const app = mountPage();
    await flushUi();

    harness.lists[0]!.emit('update:selection', new Set(['b', 'c', 'd']));
    await flushUi();
    expect(harness.selectionPanels.at(-1)!.props.trash).toBe(true);
    harness.selectionPanels.at(-1)!.emit('delete');
    await flushUi();
    expect(textOf(pageRoot)).toContain('Subject b');
    expect(dialog()?.props).toMatchObject({
      title: 'Delete 3 emails permanently?',
      description: 'This cannot be undone. These emails will be permanently removed from your Web and Mirror accounts.',
    });
    expect(harness.client.messages.trash).not.toHaveBeenCalled();
    (findButton(pageRoot, 'Delete 3 emails')!.props.onClick as () => void)();
    await flushUi();
    await flushUi();

    expect(vi.mocked(harness.client.messages.delete).mock.calls.map(([id]) => id).sort()).toEqual(['b', 'c', 'd']);
    expect(harness.router.push).toHaveBeenCalledWith({ name: 'mail', query: { folder: 'trash' } });
    expect(harness.lists[0]!.props.messages.map((summary: { id: string }) => summary.id)).toEqual(['a']);
    expect(harness.lists[0]!.props.selection.size).toBe(0);
    expect(findButton(pageRoot, 'Delete 3 emails')).toBeUndefined();
    app.unmount();
  });

  it('keeps the dialog open on only the messages that failed to delete', async () => {
    harness.route.query = { folder: 'trash' };
    vi.mocked(harness.client.trash.listMessages).mockResolvedValue(summaries('a', 'b', 'c'));
    vi.mocked(harness.client.messages.delete).mockImplementation(async (id) => { if (id === 'b') throw new Error('Forbidden'); });
    const app = mountPage();
    await flushUi();

    harness.lists[0]!.emit('delete', ['a', 'b', 'c']);
    await flushUi();
    (findButton(pageRoot, 'Delete 3 emails')!.props.onClick as () => void)();
    await flushUi();
    await flushUi();

    expect(textOf(pageRoot)).toContain('1 of 3 emails could not be deleted. Forbidden');
    expect(dialog()?.props.title).toBe('Delete permanently?');
    expect(findButton(pageRoot, 'Delete permanently')).toBeDefined();
    expect(textOf(pageRoot)).toContain('Subject b');
    app.unmount();
  });

  it('moves deleted messages to Trash at once from the inbox, closing the open one', async () => {
    harness.route = reactive({ name: 'mail-message', params: { id: 'b' }, query: {} });
    vi.mocked(harness.client.messages.list).mockResolvedValueOnce(summaries('a', 'b', 'c', 'd')).mockResolvedValue(summaries('a', 'd'));
    const app = mountPage();
    await flushUi();
    expect(harness.details.at(-1)!.props.trash).toBe(false);

    harness.lists[0]!.emit('update:selection', new Set(['b', 'c']));
    await flushUi();
    expect(harness.selectionPanels.at(-1)!.props.trash).toBe(false);
    harness.selectionPanels.at(-1)!.emit('delete');
    await flushUi();
    await flushUi();

    expect(dialog()).toBeUndefined();
    expect(harness.client.messages.trash).toHaveBeenCalledWith(['b', 'c']);
    expect(harness.client.messages.delete).not.toHaveBeenCalled();
    expect(harness.router.push).toHaveBeenCalledWith({ name: 'mail', query: {} });
    expect(harness.workspace.refreshInboxUnread).toHaveBeenCalled();
    expect(harness.lists[0]!.props.selection.size).toBe(0);
    expect(harness.lists[0]!.props.messages.map((summary: { id: string }) => summary.id)).toEqual(['a', 'd']);
    app.unmount();
  });

  it('trashes from the list, the reader, and search results in batches of at most 256', async () => {
    harness.route = reactive({ name: 'mail-message', params: { id: 'message-1' }, query: { q: 'receipt' } });
    const app = mountPage();
    await flushUi();
    expect(harness.lists[0]!.props.showEmptyTrash).toBe(true);

    harness.details.at(-1)!.emit('delete');
    await flushUi();
    expect(harness.client.messages.trash).toHaveBeenLastCalledWith(['message-1']);

    const many = Array.from({ length: 300 }, (_, index) => `message-${index}`);
    harness.lists[0]!.emit('delete', many);
    await flushUi();
    await flushUi();
    expect(vi.mocked(harness.client.messages.trash).mock.calls.slice(1).map(([ids]) => ids.length)).toEqual([256, 44]);
    expect(dialog()).toBeUndefined();
    app.unmount();
  });

  it('reports a failed move to Trash in the list and keeps the messages', async () => {
    vi.mocked(harness.client.messages.list).mockResolvedValue(summaries('a', 'b'));
    vi.mocked(harness.client.messages.trash).mockRejectedValueOnce(new Error('Viewers cannot delete mail'));
    const app = mountPage();
    await flushUi();

    harness.lists[0]!.emit('delete', ['a']);
    await flushUi();
    expect(harness.lists[0]!.props.error).toBe('Viewers cannot delete mail');
    expect(harness.lists[0]!.props.messages.map((summary: { id: string }) => summary.id)).toEqual(['a', 'b']);
    app.unmount();
  });

  it('lists Trash through its endpoint and restores messages with Move to', async () => {
    harness.route.query = { folder: 'trash' };
    vi.mocked(harness.client.trash.listMessages).mockResolvedValueOnce(summaries('a', 'b')).mockResolvedValue(summaries('b'));
    const app = mountPage();
    await flushUi();
    expect(harness.client.trash.listMessages).toHaveBeenLastCalledWith({ before: undefined, limit: 50 });
    expect(harness.client.messages.list).not.toHaveBeenCalled();

    harness.lists[0]!.emit('move', ['a'], null);
    await flushUi();
    expect(harness.client.messages.move).toHaveBeenCalledWith(['a'], null);
    expect(harness.client.trash.listMessages).toHaveBeenCalledTimes(2);
    expect(harness.lists[0]!.props.messages.map((summary: { id: string }) => summary.id)).toEqual(['b']);
    expect(harness.workspace.refreshInboxUnread).toHaveBeenCalled();
    app.unmount();
  });

  it('confirms Empty Trash with the known count, then empties and reloads the Trash', async () => {
    harness.route.query = { folder: 'trash' };
    vi.mocked(harness.client.trash.listMessages).mockResolvedValueOnce(summaries('a', 'b', 'c')).mockResolvedValue([]);
    vi.mocked(harness.client.trash.empty).mockResolvedValueOnce(3);
    const app = mountPage();
    await flushUi();
    expect(harness.lists[0]!.props.emptyTrashDisabled).toBe(false);

    harness.lists[0]!.emit('emptyTrash');
    await flushUi();
    expect(dialog()?.props).toMatchObject({
      title: 'Empty Trash?', description: 'All 3 emails in the Trash will be permanently removed from your Web and Mirror accounts. This cannot be undone.',
    });
    expect(harness.client.trash.empty).not.toHaveBeenCalled();
    (findButton(pageRoot, 'Empty Trash')!.props.onClick as () => void)();
    await flushUi();
    await flushUi();

    expect(harness.client.trash.empty).toHaveBeenCalledTimes(1);
    expect(dialog()).toBeUndefined();
    expect(harness.client.trash.listMessages).toHaveBeenCalledTimes(2);
    expect(harness.lists[0]!.props.messages).toEqual([]);
    expect(harness.lists[0]!.props.emptyTrashDisabled).toBe(true);
    expect(harness.workspace.refreshInboxUnread).toHaveBeenCalled();
    app.unmount();
  });

  it('leaves the Empty Trash count out when older pages are not loaded and keeps the dialog open on failure', async () => {
    harness.route.query = { folder: 'trash' };
    const page = Array.from({ length: 50 }, (_, index) => `message-${index}`);
    vi.mocked(harness.client.trash.listMessages).mockResolvedValue(summaries(...page));
    vi.mocked(harness.client.trash.empty).mockRejectedValueOnce(new Error('Forbidden'));
    const app = mountPage();
    await flushUi();

    harness.lists[0]!.emit('emptyTrash');
    await flushUi();
    expect(dialog()?.props.description).toBe('All emails in the Trash will be permanently removed from your Web and Mirror accounts. This cannot be undone.');
    (findButton(pageRoot, 'Empty Trash')!.props.onClick as () => void)();
    await flushUi();
    expect(textOf(pageRoot)).toContain('Forbidden');
    expect(dialog()).toBeDefined();
    app.unmount();
  });

  it('does not offer Empty Trash to viewers', async () => {
    ({ workspace: harness.workspace, emitEvent } = createWorkspaceDouble('viewer'));
    harness.route.query = { folder: 'trash' };
    const app = mountPage();
    await flushUi();
    expect(harness.lists[0]!.props.showEmptyTrash).toBe(false);
    app.unmount();
  });
});
