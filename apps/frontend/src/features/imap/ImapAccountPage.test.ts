import * as VueRuntime from 'vue'
import { transpile, ScriptTarget, ModuleKind } from 'typescript'
import { createRenderer, defineComponent, h, nextTick, onMounted, ref, ssrContextKey, type Component } from 'vue'
import { compileScript, compileTemplate, parse } from '@vue/compiler-sfc'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { ImapAccount, ImapAccountInput, ImapAccountMode, ImapAccountUpdate, ImapFolderInput, ImapLocalFolder, SandpostClient, SandpostEvent } from '@/api/contracts'
import { i18n } from '@/i18n'

const harness = vi.hoisted(() => ({ client: {} as SandpostClient, receive: null as null | ((event: SandpostEvent) => void) }))
const { primitive, primitives } = vi.hoisted(() => {

  /**
 * Create a component stand-in that renders its slot as the requested host tag.
 */
  const primitive = (tag: string) => defineComponent({

    /**
 * Render this component stand-in with the props and slots needed by the page test.
 */
    setup: (_, { attrs, slots }) => () => h(tag, attrs, slots.default?.())
  })
  return {
    primitive,

    /**
 * Build named stand-ins for each whitespace-separated UI primitive.
 */
    primitives: (names: string) => Object.fromEntries(names.split(' ').map((name) => [name, primitive(name)])),
  }
})
vi.mock('vue-router', () => ({

  /**
 * Supply the fixed account route consumed by the page.
 */
  useRoute: () => ({ params: { id: 'account-id' } }),

  /**
 * Supply a router double with a callable replace method.
 */
  useRouter: () => ({ replace: vi.fn() }),
  RouterLink: primitive('a'),
}))
vi.mock('@/runtime/client', () => ({

  /**
 * Expose the client double configured by each account-page test.
 */
  useSandpostClient: () => harness.client,
}))
vi.mock('@/features/shell/workspace', () => ({

  /**
 * Expose the event-subscription contract used by the account page.
 */
  useWorkspace: () => ({
    user: { value: { role: 'owner' } },

    /**
 * Capture the page listener and clear it when the returned disposer runs.
 */
    onMailEvent: (receive: (event: SandpostEvent) => void) => { harness.receive = receive; return () => { harness.receive = null } },
  }),
}))
vi.mock('@/components/ui/alert', () => primitives('Alert AlertAction AlertDescription'))
vi.mock('@/components/ui/badge', () => primitives('Badge'))
vi.mock('@/components/ui/button', () => ({ Button: primitive('button') }))
vi.mock('@/components/ui/card', () => primitives('Card CardAction CardContent CardDescription CardFooter CardHeader CardTitle'))
vi.mock('@/components/ui/collapsible', () => primitives('Collapsible CollapsibleContent CollapsibleTrigger'))
vi.mock('@/components/ui/dialog', () => primitives('DialogFooter'))
vi.mock('@/components/ui/dropdown-menu', () => primitives('DropdownMenu DropdownMenuContent DropdownMenuGroup DropdownMenuItem DropdownMenuSeparator DropdownMenuTrigger'))
vi.mock('@/components/ui/empty', () => primitives('Empty EmptyContent EmptyDescription EmptyHeader EmptyMedia EmptyTitle'))
vi.mock('@/components/ui/field', () => primitives('Field FieldDescription FieldError FieldGroup FieldLabel'))
vi.mock('@/components/ui/input', () => ({ Input: primitive('input') }))
vi.mock('@/components/ui/input-group', () => primitives('InputGroup InputGroupAddon InputGroupButton InputGroupInput'))
vi.mock('@/components/ui/item', () => primitives('Item ItemActions ItemContent ItemDescription ItemGroup ItemMedia ItemTitle'))
vi.mock('@/components/ui/skeleton', () => primitives('Skeleton'))
vi.mock('@/components/ui/spinner', () => primitives('Spinner'))
vi.mock('@/components/ui/toggle-group', () => primitives('ToggleGroup ToggleGroupItem'))
vi.mock('@/components/BusyDialog.vue', () => ({
  default: defineComponent({
    props: ['open', 'title', 'description', 'busy'],

    /**
   * Render dialog details and slot content only while the stub is open.
   */
    setup: (props, { slots }) => () => props.open ? h('dialog', {}, [h('h2', props.title), h('p', props.description), slots.default?.()]) : null,
  })
}))
vi.mock('@/features/query/QueryEditor.vue', () => ({
  default: defineComponent({
    props: ['modelValue', 'valid'], emits: ['update:modelValue', 'update:valid'],

    /**
   * Mark the query stub valid after mounting and render its host marker.
   */
    setup: (_, { emit }) => { onMounted(() => emit('update:valid', true)); return () => h('query') },
  })
}))
vi.mock('@/features/imap/ImapCredentialsDialog.vue', () => ({ default: primitive('credentials') }))
vi.mock('@/features/imap/ImapServerNotice.vue', () => ({ default: primitive('server') }))
vi.mock('@/features/imap/ImapSetupInstructions.vue', () => ({ default: primitive('instructions') }))

import ImapAccountPage from './ImapAccountPage.vue'
import pageSource from './ImapAccountPage.vue?raw'
import CreateDialog from '@/features/imap/ImapAccountCreateDialog.vue'
import createSource from '@/features/imap/ImapAccountCreateDialog.vue?raw'
import FolderDialog from '@/features/imap/ImapFolderDialog.vue'
import folderSource from '@/features/imap/ImapFolderDialog.vue?raw'
import AccountDialogs from '@/features/imap/ImapAccountDialogs.vue'
import dialogsSource from '@/features/imap/ImapAccountDialogs.vue?raw'

/**
 * Compile client templates for the lightweight renderer used by these Node tests.
 */
function attachRender(component: Component, source: string): void {
  const descriptor = parse(source).descriptor
  const compiled = compileTemplate({
    source: descriptor.template!.content, filename: 'test.vue', id: 'imap-page',
    compilerOptions: { bindingMetadata: compileScript(descriptor, { id: 'imap-page' }).bindings },
  })
  const render = new Function('VueRuntime', transpile(compiled.code, { target: ScriptTarget.ES2022, module: ModuleKind.ESNext })
    .replace(/^import \{([\s\S]*?)\} from "vue"\s*;?\s*/m, (_, bindings: string) => `const {${bindings.replace(/\s+as\s+/g, ': ')}} = VueRuntime;`)
    .replace('export function render', 'return function render'))(VueRuntime)
  Object.assign(component, { render })
}
for (const [component, source] of [[ImapAccountPage, pageSource], [CreateDialog, createSource], [FolderDialog, folderSource], [AccountDialogs, dialogsSource]] as const) attachRender(component, source)

interface HostNode { type: string; props: Record<string, any>; children: HostNode[]; parent: HostNode | null; text?: string }

/**
 * Create one renderer node without depending on a DOM implementation.
 */
function node(type: string, text?: string): HostNode { return { type, text, props: {}, children: [], parent: null } }
const renderer = createRenderer<HostNode, HostNode>({

  /**
   * Create inspectable host elements for the page renderer.
   */
  createElement: (type) => node(type),

  /**
   * Create inspectable text nodes for rendered content.
   */
  createText: (text) => node('#text', text),

  /**
   * Preserve inactive Vue branches as inspectable comment nodes.
   */
  createComment: (text) => node('#comment', text),

  /**
   * Replace the content stored in a text node.
   */
  setText: (item, text) => { item.text = text },

  /**
   * Replace element text and clear its previous child nodes.
   */
  setElementText: (item, text) => { item.text = text; item.children = [] },

  /**
   * Record patched attributes and event handlers for test assertions.
   */
  patchProp: (item, key, _old, value) => { item.props[key] = value },

  /**
   * Move a node before its anchor, detaching it from any previous parent.
   */
  insert: (item, parent, anchor) => {
    if (item.parent) item.parent.children.splice(item.parent.children.indexOf(item), 1)
    const index = anchor ? parent.children.indexOf(anchor) : -1
    parent.children.splice(index < 0 ? parent.children.length : index, 0, item); item.parent = parent
  },

  /**
   * Remove a node from its current parent and clear its parent reference.
   */
  remove: (item) => { item.parent?.children.splice(item.parent.children.indexOf(item), 1); item.parent = null },

  /**
   * Return the host node's parent for renderer traversal.
   */
  parentNode: (item) => item.parent,

  /**
   * Return the next sibling in the current parent's child list.
   */
  nextSibling: (item) => item.parent?.children[item.parent.children.indexOf(item) + 1] ?? null,

  /**
   * Insert compiled static markup as one inspectable child node.
   */
  insertStaticContent: (text, parent) => { const item = node('#static', text); item.parent = parent; parent.children.push(item); return [item, item] },
})
let root: HostNode
let unmount: (() => void) | undefined
let account: ImapAccount
const server = { enabled: true, host: 'localhost', port: 1143, security: 'none' as const, listenAddress: '127.0.0.1:1143' }
const initialFolder: ImapLocalFolder = {
  id: 'smart-id', accountId: 'account-id', kind: 'smart', definitionId: null,
  name: 'Local query', filter: 'true', includeInInbox: true, createdAt: 1, updatedAt: 1
}

/**
 * Read all text in a rendered subtree, excluding inactive branches.
 */
function text(item: HostNode = root): string { return item.type === '#comment' ? '' : (item.text ?? '') + item.children.map((child) => text(child)).join('') }

/**
 * Locate one element by its properties or text.
 */
function find(predicate: (item: HostNode) => boolean, item: HostNode = root): HostNode | undefined {
  if (predicate(item)) return item
  for (const child of item.children) { const found = find(predicate, child); if (found) return found }
}

/**
 * Return the named visible button.
 */
function button(label: string): HostNode { return find((item) => item.type === 'button' && text(item).trim() === label)! }

/**
 * Drive an input's real model handler.
 */
function fill(id: string, value: string): void { find((item) => item.props.id === id)!.props['onUpdate:modelValue'](value) }

/**
 * Submit the form containing one input, using Vue's real prevent modifier.
 */
function submit(inputId: string): void {
  find((item) => item.type === 'form' && Boolean(find((child) => child.props.id === inputId, item)))!.props.onSubmit({

    /**
     * Provide the event method expected by Vue form submission handling.
     */
    preventDefault() { }
  })
}

/**
 * Flush chained client promises and Vue updates.
 */
async function flush(): Promise<void> { for (let index = 0; index < 8; index += 1) { await Promise.resolve(); await nextTick() } }

/**
 * Mount the actual page template against the lightweight renderer.
 */
function mount(component: Component = ImapAccountPage): void {
  root = node('root'); const app = renderer.createApp(component); app.use(i18n); app.provide(ssrContextKey, { modules: new Set() }); app.mount(root); unmount = () => app.unmount()
}

/**
 * Return only the folder state visible in the server's active mode.
 */
function details(): ImapAccount { return structuredClone({ ...account, folders: account.mode === 'independent' ? account.folders : [] }) }

beforeEach(() => {
  account = {
    id: 'account-id', ownerId: 'owner', name: 'Mail', username: 'mail', enabled: true, mode: 'mirror',
    createdAt: 1, updatedAt: 1, lastUsedAt: null, linkedViews: [], folders: [initialFolder], mailboxes: []
  }
  harness.client = {
    imap: {
      server: vi.fn(async () => server), get: vi.fn(async () => details()),
      update: vi.fn(async (_id: string, changes: ImapAccountUpdate) => { Object.assign(account, changes); return details() }),
      create: vi.fn(async (input: ImapAccountInput) => ({ account: { ...details(), mode: input.mode ?? 'mirror' }, password: 'issued-once', server })),
      folders: {
        create: vi.fn(async (_accountId: string, input: ImapFolderInput) => { const folder = { ...initialFolder, ...input, id: 'new-smart' }; account.folders.push(folder); return folder }),
        update: vi.fn(async (_accountId: string, id: string, input: ImapFolderInput) => { const folder = account.folders.find((candidate) => candidate.id === id)!; Object.assign(folder, input); return folder }),
        delete: vi.fn(async (_accountId: string, id: string) => { account.folders = account.folders.filter((folder) => folder.id !== id) }),
      },
    }
  } as unknown as SandpostClient
})
afterEach(() => { unmount?.(); harness.receive = null })

describe('IMAP account mode UI', () => {
  it('saves explicit mode, preserves drafts across events, and gates Smart Folder editing', async () => {
    mount(); await flush()
    expect(button('Add Smart Folder')).toBeUndefined()
    const toggle = find((item) => item.type === 'ToggleGroup')!
    toggle.props['onUpdate:modelValue']('independent'); await flush()
    harness.receive?.({ type: 'resync' }); await flush()
    expect(toggle.props['model-value']).toBe('independent')
    toggle.props['onUpdate:modelValue'](undefined); await flush()
    expect(toggle.props['model-value']).toBe('independent')
    find((item) => item.props.id === 'imap-mailboxes-form')!.props.onSubmit({

      /**
       * Provide the event method expected by Vue form submission handling.
       */
      preventDefault() { }
    }); await flush()
    expect(harness.client.imap.update).toHaveBeenCalledWith('account-id', { mode: 'independent' })
    expect(button('Add Smart Folder')).toBeDefined()
    expect(text()).toContain('Reconnect them')
    expect(text()).not.toContain('Include in INBOX')
    expect(text()).not.toContain('Selected folders')
    button('Delete account').props.onClick(); await flush()
    expect(text()).toContain('Independent retained history')
    expect(text()).toContain('permanently removed')
  })

  it('creates, edits and deletes Smart Folders only through the Independent account endpoints', async () => {
    account.mode = 'independent'
    mount(); await flush()
    button('Add Smart Folder').props.onClick(); await flush()
    fill('imap-folder-name', 'New local'); submit('imap-folder-name'); await flush()
    expect(harness.client.imap.folders.create).toHaveBeenCalledWith('account-id', { name: 'New local', filter: '' })
    find((item) => item.props['aria-label'] === 'Edit New local')!.props.onClick(); await flush()
    fill('imap-folder-name', 'Changed local'); submit('imap-folder-name'); await flush()
    expect(harness.client.imap.folders.update).toHaveBeenCalledWith('account-id', 'new-smart', { name: 'Changed local', filter: '' })
    find((item) => item.props['aria-label'] === 'Delete Changed local')!.props.onClick(); await flush()
    button('Delete folder').props.onClick(); await flush()
    expect(harness.client.imap.folders.delete).toHaveBeenCalledWith('account-id', 'new-smart')
  })

  it('keeps Smart Folder controls hidden after a failed mode save', async () => {
    mount(); await flush()
    find((item) => item.type === 'ToggleGroup')!.props['onUpdate:modelValue']('independent'); await flush()
    vi.mocked(harness.client.imap.update).mockRejectedValueOnce(new Error('Mode update failed'))
    find((item) => item.props.id === 'imap-mailboxes-form')!.props.onSubmit({

      /**
       * Provide the event method expected by Vue form submission handling.
       */
      preventDefault() { }
    }); await flush()
    expect(text()).toContain('Mode update failed')
    expect(button('Add Smart Folder')).toBeUndefined()
  })

  it.each(['mirror', 'independent'] as ImapAccountMode[])('creates %s credentials without view or exposure selection', async (mode) => {
    const open = ref(false)
    const created = vi.fn()
    mount(defineComponent({

      /**
       * Render this component stand-in with the props and slots needed by the page test.
       */
      setup: () => () => h(CreateDialog, {
        open: open.value,

        /**
         * Synchronize the dialog open model with the test ref.
         */
        'onUpdate:open': (value) => { open.value = value }, onCreated: created
      })
    }))
    open.value = true; await flush()
    expect(find((item) => item.type === 'ToggleGroup')!.props['model-value']).toBe('mirror')
    find((item) => item.type === 'ToggleGroup')!.props['onUpdate:modelValue'](mode); await flush()
    fill('imap-account-name', ' My client '); submit('imap-account-name'); await flush()
    expect(harness.client.imap.create).toHaveBeenCalledWith({ name: 'My client', username: null, mode })
    expect(created).toHaveBeenCalledOnce()
    expect(open.value).toBe(false)
  })
})
