import * as VueRuntime from 'vue'
import { createRenderer, defineComponent, h, nextTick, ref, ssrContextKey, type Component } from 'vue'
import { compileScript, compileTemplate, parse } from '@vue/compiler-sfc'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { SandpostClient, Scope, SessionUser, UserAccount } from '@/api/contracts'
import { i18n } from '@/i18n'

const harness = vi.hoisted(() => ({
  client: {} as SandpostClient,
  context: {} as { isGlobalAdmin: { value: boolean }; user: { value: SessionUser | null } },
  router: { replace: vi.fn(async () => undefined) },
}))

const { primitive, primitives } = vi.hoisted(() => {
  /**
   * Create a visual component stand-in that renders its slot as a host element.
   */
  const primitive = (tag: string) => defineComponent({
    inheritAttrs: false,
    props: ['modelValue', 'checked', 'value'],
    emits: ['update:modelValue'],
    setup: (props, { attrs, emit, slots }) => () => h(tag, {
      ...attrs,
      ...(props.modelValue === undefined ? {} : tag === 'input' && String(attrs.id ?? '').startsWith('user-scope-') ? { checked: props.modelValue } : { value: props.modelValue }),
      ...(props.checked === undefined ? {} : { checked: props.checked }),
      onInput: (event: Event) => emit('update:modelValue', (event.target as HTMLInputElement).value),
      onChange: (event: Event) => emit('update:modelValue', tag === 'select' ? (event.target as HTMLSelectElement).value : (event.target as HTMLInputElement).checked),
    }, slots.default?.()),
  })

  return {
    primitive,

    /**
     * Build named visual component stand-ins for the page's UI imports.
     */
    primitives: (names: string) => Object.fromEntries(names.split(' ').map((name) => [name, primitive(name === 'NativeSelect' ? 'select' : name === 'Checkbox' ? 'input' : 'div')])),
  }
})

vi.mock('vue-router', () => ({
  /**
   * Expose the router double configured by each user-page test.
   */
  useRouter: () => harness.router,
}))
vi.mock('@/runtime/client', () => ({
  /**
   * Expose the client double configured by each user-page test.
   */
  useSandpostClient: () => harness.client,
}))
vi.mock('@/features/admin/context', () => ({
  /**
   * Expose the owner identity and permission configured by each user-page test.
   */
  useAdminContext: () => harness.context,
}))

vi.mock('@lucide/vue', () => ({
  ChevronRightIcon: primitive('svg'), PencilIcon: primitive('svg'), PlusIcon: primitive('svg'), SearchIcon: primitive('svg'), Trash2Icon: primitive('svg'), UsersIcon: primitive('svg'),
}))
vi.mock('@/components/ui/alert', () => primitives('Alert AlertDescription'))
vi.mock('@/components/ui/badge', () => primitives('Badge'))
vi.mock('@/components/ui/button', () => ({ Button: primitive('button') }))
vi.mock('@/components/ui/checkbox', () => ({ Checkbox: primitive('input') }))
vi.mock('@/components/ui/command', () => primitives('Command CommandEmpty CommandGroup CommandInput CommandItem CommandList'))
vi.mock('@/components/ui/dialog', () => ({ DialogFooter: primitive('div') }))
vi.mock('@/components/ui/dropdown-menu', () => primitives('DropdownMenu DropdownMenuContent DropdownMenuItem DropdownMenuTrigger'))
vi.mock('@/components/ui/empty', () => primitives('Empty EmptyDescription EmptyHeader EmptyMedia EmptyTitle'))
vi.mock('@/components/ui/field', () => primitives('Field FieldContent FieldDescription FieldError FieldGroup FieldLabel FieldLegend FieldSet'))
vi.mock('@/components/ui/input', () => ({ Input: primitive('input') }))
vi.mock('@/components/ui/input-group', () => ({ InputGroup: primitive('div'), InputGroupAddon: primitive('div'), InputGroupInput: primitive('input') }))
vi.mock('@/components/ui/native-select', () => ({ NativeSelect: primitive('select'), NativeSelectOption: primitive('option') }))
vi.mock('reka-ui', async () => {
  const { computed, defineComponent, h, inject, provide } = await import('vue')
  const treeContextKey = Symbol('test-tree-context')

  /**
   * Render Reka's flattened tree slot and expose controlled selection and expansion to TreeItems.
   */
  const TreeRoot = defineComponent({
    inheritAttrs: false,
    props: ['items', 'getKey', 'getChildren', 'expanded', 'modelValue', 'multiple', 'selectionBehavior', 'disabled'],
    emits: ['update:modelValue', 'update:expanded'],
    setup: (props, { attrs, emit, slots }) => {
      const expanded = computed(() => props.expanded ?? [])
      const selected = computed(() => Array.isArray(props.modelValue) ? props.modelValue : props.modelValue ? [props.modelValue] : [])
      const context = {
        getKey: props.getKey,
        getChildren: props.getChildren,
        isExpanded: (value: any) => expanded.value.includes(props.getKey(value)),
        isSelected: (value: any) => selected.value.some((item: any) => props.getKey(item) === props.getKey(value)),
        toggle: (value: any) => {
          const key = props.getKey(value)
          emit('update:expanded', expanded.value.includes(key) ? expanded.value.filter((identifier: string) => identifier !== key) : [...expanded.value, key])
        },
        select: (value: any) => {
          const key = props.getKey(value)
          const next = context.isSelected(value)
            ? selected.value.filter((item: any) => props.getKey(item) !== key)
            : [...selected.value, value]
          emit('update:modelValue', props.multiple ? next : next[0])
        },
      }
      provide(treeContextKey, context)

      /**
       * Flatten only expanded branches while retaining each scope's hierarchy level.
       */
      function flattenItems(items: any[], level = 1): any[] {
        return items.flatMap((value, index) => {
          const key = props.getKey(value)
          const children = props.getChildren(value)
          const row = { _id: key, index, value, level, hasChildren: Boolean(children), bind: { value, level, 'aria-setsize': items.length, 'aria-posinset': index + 1 } }
          return children && context.isExpanded(value) ? [row, ...flattenItems(children, level + 1)] : [row]
        })
      }

      return () => h('ul', { ...attrs, role: 'tree' }, slots.default?.({ flattenItems: flattenItems(props.items ?? []) }))
    },
  })

  /**
   * Render a tree row with the selection, expansion, and toggle-slot state used by UsersPage.
   */
  const TreeItem = defineComponent({
    inheritAttrs: false,
    props: ['value', 'level'],
    setup: (props, { attrs, slots }) => {
      const context = inject<any>(treeContextKey)!
      return () => {
        const children = context.getChildren(props.value)
        return h('li', {
          ...attrs,
          role: 'treeitem',
          'aria-level': props.level,
          'aria-expanded': children ? context.isExpanded(props.value) : undefined,
          'aria-selected': context.isSelected(props.value),
          onClick: () => context.select(props.value),
        }, slots.default?.({
          isExpanded: context.isExpanded(props.value),
          isSelected: context.isSelected(props.value),
          isIndeterminate: undefined,
          isDisabled: false,
          handleToggle: () => context.toggle(props.value),
          handleSelect: () => context.select(props.value),
        }))
      }
    },
  })

  return { TreeRoot, TreeItem }
})
vi.mock('@/components/ui/skeleton', () => ({ Skeleton: primitive('div') }))
vi.mock('@/components/ui/spinner', () => ({ Spinner: primitive('span') }))
vi.mock('@/components/ui/table', () => primitives('Table TableBody TableCell TableHead TableHeader TableRow'))
vi.mock('@/components/BusyDialog.vue', () => ({
  default: defineComponent({
    props: ['open', 'title', 'description', 'busy'],

    /**
     * Render dialog content while the page's controlled open prop is true.
     */
    setup: (props, { slots }) => () => props.open ? h('dialog', {}, [h('h2', props.title), slots.default?.()]) : null,
  }),
}))

import UsersPage from './UsersPage.vue'
import pageSource from './UsersPage.vue?raw'

/**
 * Render transition-group children directly because the custom test renderer has no DOM classes.
 */
const passthroughTransitionGroup = defineComponent({ setup: (_props, { slots }) => () => slots.default?.() })

/**
 * Compile the UsersPage template with the script setup bindings for the custom renderer.
 */
function attachRender(component: Component, source: string): void {
  const descriptor = parse(source, { filename: 'UsersPage.vue' }).descriptor
  const compiled = compileTemplate({
    source: descriptor.template!.content,
    filename: 'UsersPage.vue',
    id: 'users-page-test',
    compilerOptions: { bindingMetadata: compileScript(descriptor, { id: 'users-page-test' }).bindings },
  })
  const render = new Function('VueRuntime', compiled.code
    .replace(/^import \{([\s\S]*?)\} from "vue"\s*;?\s*/m, (_statement, bindings: string) =>
      `const {${bindings.replace(/\s+as\s+/g, ': ')}} = VueRuntime;`)
    .replace('export function render', 'return function render'))({ ...VueRuntime, TransitionGroup: passthroughTransitionGroup })
  Object.assign(component, { render })
}
attachRender(UsersPage, pageSource)

interface HostNode { type: string; props: Record<string, any>; children: HostNode[]; parent: HostNode | null; text?: string }

/**
 * Create a host node for the renderer that runs without a browser DOM.
 */
function node(type: string, text?: string): HostNode { return { type, text, props: {}, children: [], parent: null } }

const renderer = createRenderer<HostNode, HostNode>({
  /**
   * Create an inspectable element host node.
   */
  createElement: (type) => node(type),

  /**
   * Create an inspectable text host node.
   */
  createText: (text) => node('#text', text),

  /**
   * Create an inspectable comment host node for inactive branches.
   */
  createComment: (text) => node('#comment', text),

  /**
   * Update a text node after Vue patches it.
   */
  setText: (item, text) => { item.text = text },

  /**
   * Replace element text and clear its children.
   */
  setElementText: (item, text) => { item.text = text; item.children = [] },

  /**
   * Record rendered attributes and event handlers for assertions.
   */
  patchProp: (item, key, _old, value) => { item.props[key] = value },

  /**
   * Place a rendered node under its current parent and optional anchor.
   */
  insert: (item, parent, anchor) => {
    if (item.parent) item.parent.children.splice(item.parent.children.indexOf(item), 1)
    const index = anchor ? parent.children.indexOf(anchor) : -1
    parent.children.splice(index < 0 ? parent.children.length : index, 0, item)
    item.parent = parent
  },

  /**
   * Remove a rendered node from its current parent.
   */
  remove: (item) => { if (item.parent) item.parent.children.splice(item.parent.children.indexOf(item), 1); item.parent = null },

  /**
   * Return the parent recorded for a rendered host node.
   */
  parentNode: (item) => item.parent,

  /**
   * Return the next sibling recorded for a rendered host node.
   */
  nextSibling: (item) => item.parent?.children[item.parent.children.indexOf(item) + 1] ?? null,

  /**
   * Insert compiled static markup as an inspectable node.
   */
  insertStaticContent: (content, parent) => { const item = node('#static', content); item.parent = parent; parent.children.push(item); return [item, item] },
})

const owner: SessionUser = { id: 'owner-id', name: 'Owner', email: 'owner@example.test', role: 'owner' }
const ownerAccount: UserAccount = { ...owner, mailAccess: 'all' }
const existingUser: UserAccount = { id: 'member-id', name: 'Member', email: 'member@example.test', role: 'member', mailAccess: 'scoped' }
const secondUser: UserAccount = { id: 'second-id', name: 'Second member', email: 'second@example.test', role: 'member', mailAccess: 'scoped' }
const scopes: Scope[] = [
  { id: 'scope-three', parentId: null, name: 'Gamma Archive', description: null, filter: 'true', position: 2, policyVersion: 1 },
  { id: 'scope-one', parentId: null, name: 'Alpha Team', description: null, filter: 'true', position: 0, policyVersion: 1 },
  { id: 'scope-two', parentId: 'scope-one', name: 'Beta Projects', description: null, filter: 'true', position: 1, policyVersion: 1 },
]
let root: HostNode
let app: ReturnType<typeof renderer.createApp> | undefined
let accountList: UserAccount[]
let assignments: Map<string, Set<string>>

/**
 * Return visible text beneath a host node, skipping inactive Vue branches.
 */
function textOf(item: HostNode = root): string { return item.type === '#comment' ? '' : (item.text ?? '') + item.children.map((child) => textOf(child)).join('') }

/**
 * Find the first rendered node matching a predicate.
 */
function find(predicate: (item: HostNode) => boolean, item: HostNode = root): HostNode | undefined {
  if (predicate(item)) return item
  for (const child of item.children) { const match = find(predicate, child); if (match) return match }
}

/**
 * Find a visible button whose rendered text matches the requested label.
 */
function button(label: string): HostNode | undefined { return find((item) => item.type === 'button' && textOf(item).trim() === label) }

/**
 * Find an account edit button by the accessible name rendered by UsersPage.
 */
function editButton(name: string): HostNode | undefined { return find((item) => item.type === 'button' && item.props['aria-label'] === `Edit ${name}`) }

/**
 * Find one scope checkbox by its stable scope identifier.
 */
function scopeCheckbox(identifier: string): HostNode | undefined { return find((item) => item.props.id === `user-scope-${identifier}`) }

/**
 * Drive the model update handler attached to the input with the requested placeholder.
 */
function fillPlaceholder(placeholder: string, value: string): void { find((item) => item.type === 'input' && item.props.placeholder === placeholder)!.props.onInput({ target: { value } }) }

/**
 * Submit the form currently rendered in the user dialog.
 */
function submitForm(): void { find((item) => item.type === 'form')!.props.onSubmit({ preventDefault() {} }) }

/**
 * Flush pending client promises and Vue rendering after user actions.
 */
async function flush(): Promise<void> { for (let index = 0; index < 8; index += 1) { await Promise.resolve(); await nextTick() } }

/**
 * Create a promise whose resolution is controlled by a race-condition test.
 */
function deferred<T>(): { promise: Promise<T>; resolve: (value: T) => void } {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((complete) => { resolve = complete })
  return { promise, resolve }
}

/**
 * Mount UsersPage in the custom renderer with its admin identity and translations.
 */
function mount(): void {
  root = node('root')
  app = renderer.createApp(UsersPage)
  app.use(i18n)
  app.provide(ssrContextKey, { modules: new Set() })
  app.mount(root)
}

beforeEach(() => {
  accountList = [ownerAccount, { ...existingUser }, { ...secondUser }]
  assignments = new Map(scopes.map((scope) => [scope.id, new Set(scope.id === 'scope-one' ? [existingUser.id] : scope.id === 'scope-two' ? [secondUser.id] : [])]))
  harness.router.replace.mockClear()
  harness.context = { isGlobalAdmin: ref(true), user: ref(owner) }
  harness.client = {
    admin: {
      users: {
        list: vi.fn(async () => [...accountList]),
        create: vi.fn(async (input) => { const created = { ...input, id: 'created-id' }; accountList.push(created); return created }),
        update: vi.fn(async (id, changes) => { const updated = accountList.find((account) => account.id === id)!; Object.assign(updated, changes); return updated }),
        delete: vi.fn(async (id) => { accountList = accountList.filter((account) => account.id !== id) }),
      },
      scopes: {
        list: vi.fn(async () => [...scopes]),
        assignments: vi.fn(async (scopeId) => [...(assignments.get(scopeId) ?? [])]),
        assign: vi.fn(async (scopeId, userId) => { assignments.get(scopeId)?.add(userId) }),
        unassign: vi.fn(async (scopeId, userId) => { assignments.get(scopeId)?.delete(userId) }),
      },
    },
  } as unknown as SandpostClient
})

afterEach(() => { app?.unmount(); app = undefined })

describe('UsersPage scoped access editing', () => {
  it('loads scopes on open, preselects memberships, and keeps multiple toggles through search', async () => {
    mount(); await flush()
    editButton('Member')!.props.onClick(); await flush()

    expect(harness.client.admin.scopes.list).toHaveBeenCalledOnce()
    expect(harness.client.admin.scopes.assignments).toHaveBeenCalledTimes(scopes.length)
    expect(textOf()).toContain('Alpha Team')
    expect(scopeCheckbox('scope-two')).toBeDefined()
    find((item) => item.type === 'button' && item.props['aria-label'] === 'Collapse Alpha Team')!.props.onClick({ stopPropagation() {} })
    await flush()
    expect(scopeCheckbox('scope-two')).toBeUndefined()

    fillPlaceholder('Search scopes by name…', 'Beta'); await flush()
    expect(textOf()).toContain('Beta Projects')
    expect(scopeCheckbox('scope-two')).toBeDefined()
    expect(scopeCheckbox('scope-one')).toBeDefined()
    expect(scopeCheckbox('scope-three')).toBeUndefined()
    scopeCheckbox('scope-two')!.props.onChange({ target: { checked: true } })
    fillPlaceholder('Search scopes by name…', 'Alpha'); await flush()
    expect(scopeCheckbox('scope-one')!.props.checked).toBe(true)
    scopeCheckbox('scope-one')!.props.onChange({ target: { checked: false } })
    fillPlaceholder('Search scopes by name…', 'Beta'); await flush()
    expect(scopeCheckbox('scope-two')!.props.checked).toBe(true)
  })

  it('collapses a branch without clearing a selected child', async () => {
    assignments.get('scope-two')!.add(existingUser.id)
    mount(); await flush(); editButton('Member')!.props.onClick(); await flush()
    expect(scopeCheckbox('scope-two')!.props.checked).toBe(true)
    find((item) => item.type === 'button' && item.props['aria-label'] === 'Collapse Alpha Team')!.props.onClick({ stopPropagation() {} })
    await flush()
    expect(scopeCheckbox('scope-two')).toBeUndefined()
    expect(textOf()).toContain('2 scopes selected')
    expect(find((item) => item.props['data-selected-descendant'] !== undefined)).toBeDefined()
    find((item) => item.type === 'button' && item.props['aria-label'] === 'Expand Alpha Team')!.props.onClick({ stopPropagation() {} })
    await flush()
    expect(scopeCheckbox('scope-two')!.props.checked).toBe(true)
  })

  it('keeps parent and child memberships independent', async () => {
    mount(); await flush(); editButton('Member')!.props.onClick(); await flush()
    expect(scopeCheckbox('scope-one')!.props.checked).toBe(true)
    expect(scopeCheckbox('scope-two')!.props.checked).toBe(false)
    scopeCheckbox('scope-two')!.props.onChange({ target: { checked: true } })
    await flush()
    expect(scopeCheckbox('scope-one')!.props.checked).toBe(true)
    expect(scopeCheckbox('scope-two')!.props.checked).toBe(true)
    scopeCheckbox('scope-one')!.props.onChange({ target: { checked: false } })
    await flush()
    expect(scopeCheckbox('scope-one')!.props.checked).toBe(false)
    expect(scopeCheckbox('scope-two')!.props.checked).toBe(true)
  })

  it('cancels without changing memberships or account fields', async () => {
    mount(); await flush(); editButton('Member')!.props.onClick(); await flush()
    button('Cancel')!.props.onClick(); await flush()
    expect([...assignments.get('scope-one')!]).toEqual([existingUser.id])
    expect(harness.client.admin.scopes.assign).not.toHaveBeenCalled()
    expect(harness.client.admin.scopes.unassign).not.toHaveBeenCalled()
    expect(harness.client.admin.users.update).not.toHaveBeenCalled()
  })

  it('preserves assignments when mail access changes to All', async () => {
    mount(); await flush(); editButton('Member')!.props.onClick(); await flush()
    const access = find((item) => item.type === 'select' && item.props.id === 'user-mail-access')!
    access.props.onChange({ target: { value: 'all' } })
    await flush(); submitForm(); await flush()
    expect(harness.client.admin.users.update).toHaveBeenCalledWith(existingUser.id, { mailAccess: 'all' })
    expect(assignments.get('scope-one')).toEqual(new Set([existingUser.id]))
    expect(find((item) => item.props.id === 'user-scopes')).toBeUndefined()
    expect(harness.client.admin.scopes.assign).not.toHaveBeenCalled()
    expect(harness.client.admin.scopes.unassign).not.toHaveBeenCalled()
  })

  it('does not save when assignments fail to load', async () => {
    vi.mocked(harness.client.admin.scopes.assignments).mockRejectedValueOnce(new Error('offline'))
    mount(); await flush(); editButton('Member')!.props.onClick(); await flush()
    submitForm(); await flush()
    expect(harness.client.admin.users.update).not.toHaveBeenCalled()
    expect(harness.client.admin.scopes.assign).not.toHaveBeenCalled()
    expect(harness.client.admin.scopes.unassign).not.toHaveBeenCalled()
  })

  it('does not save when scope definitions fail to load', async () => {
    vi.mocked(harness.client.admin.scopes.list).mockRejectedValueOnce(new Error('offline'))
    mount(); await flush(); editButton('Member')!.props.onClick(); await flush()
    submitForm(); await flush()
    expect(harness.client.admin.users.update).not.toHaveBeenCalled()
    expect(harness.client.admin.scopes.assign).not.toHaveBeenCalled()
    expect(harness.client.admin.scopes.unassign).not.toHaveBeenCalled()
  })

  it('writes self memberships before updating the account and revoking its session', async () => {
    harness.context = { isGlobalAdmin: ref(true), user: ref({ ...existingUser, role: 'owner' }) }
    accountList[1] = { ...existingUser, role: 'owner' }
    const order: string[] = []
    vi.mocked(harness.client.admin.scopes.assign).mockImplementation(async () => { order.push('assign') })
    vi.mocked(harness.client.admin.users.update).mockImplementation(async () => { order.push('update'); return existingUser })
    harness.router.replace.mockImplementation(async () => { order.push('logout') })
    mount(); await flush(); editButton('Member')!.props.onClick(); await flush()
    find((item) => item.props.id === 'user-name')!.props.onInput({ target: { value: 'Member updated' } })
    scopeCheckbox('scope-two')!.props.onChange({ target: { checked: true } })
    submitForm(); await flush()
    expect(order).toEqual(['assign', 'update', 'logout'])
  })

  it('creates a new account once and then assigns selected scopes', async () => {
    mount(); await flush(); button('New user')!.props.onClick(); await flush()
    find((item) => item.props.id === 'user-name')!.props.onInput({ target: { value: 'New member' } })
    find((item) => item.props.id === 'user-email')!.props.onInput({ target: { value: 'new@example.test' } })
    find((item) => item.props.id === 'user-password')!.props.onInput({ target: { value: 'long-enough-password' } })
    scopeCheckbox('scope-two')!.props.onChange({ target: { checked: true } })
    submitForm(); await flush()
    expect(harness.client.admin.users.create).toHaveBeenCalledOnce()
    expect(assignments.get('scope-two')).toContain('created-id')
  })

  it('retries only failed membership writes without recreating a new account', async () => {
    let assignmentAttempts = 0
    vi.mocked(harness.client.admin.scopes.assign).mockImplementation(async (scopeId, userId) => {
      assignmentAttempts += 1
      if (assignmentAttempts === 2) throw new Error('offline')
      assignments.get(scopeId)?.add(userId)
    })
    mount(); await flush(); button('New user')!.props.onClick(); await flush()
    find((item) => item.props.id === 'user-name')!.props.onInput({ target: { value: 'New member' } })
    find((item) => item.props.id === 'user-email')!.props.onInput({ target: { value: 'new@example.test' } })
    find((item) => item.props.id === 'user-password')!.props.onInput({ target: { value: 'long-enough-password' } })
    scopeCheckbox('scope-two')!.props.onChange({ target: { checked: true } })
    scopeCheckbox('scope-three')!.props.onChange({ target: { checked: true } })
    submitForm(); await flush()
    expect(harness.client.admin.users.create).toHaveBeenCalledOnce()
    expect(textOf()).toContain('Offline')
    submitForm(); await flush()
    expect(harness.client.admin.users.create).toHaveBeenCalledOnce()
    expect(assignments.get('scope-two')).toContain('created-id')
    expect(assignments.get('scope-three')).toContain('created-id')
    expect(harness.client.admin.scopes.assign).toHaveBeenCalledTimes(3)
    expect(harness.client.admin.scopes.assign).toHaveBeenNthCalledWith(3, 'scope-three', 'created-id')
  })

  it('does not update account fields when only memberships change', async () => {
    mount(); await flush(); editButton('Member')!.props.onClick(); await flush()
    scopeCheckbox('scope-two')!.props.onChange({ target: { checked: true } })
    scopeCheckbox('scope-one')!.props.onChange({ target: { checked: false } })
    submitForm(); await flush()
    expect(harness.client.admin.scopes.assign).toHaveBeenCalledWith('scope-two', existingUser.id)
    expect(harness.client.admin.scopes.unassign).toHaveBeenCalledWith('scope-one', existingUser.id)
    expect(harness.client.admin.users.update).not.toHaveBeenCalled()
  })

  it('does not expose link check permissions in account forms', async () => {
    mount(); await flush(); button('New user')!.props.onClick(); await flush()
    expect(textOf()).not.toContain('Check links in an email')
    expect(textOf()).not.toContain('Link checks')
    find((item) => item.props.id === 'user-name')!.props.onInput({ target: { value: 'Checker' } })
    find((item) => item.props.id === 'user-email')!.props.onInput({ target: { value: 'checker@example.test' } })
    find((item) => item.props.id === 'user-password')!.props.onInput({ target: { value: 'long-enough-password' } })
    find((item) => item.props.id === 'user-mail-access')!.props.onChange({ target: { value: 'all' } })
    submitForm(); await flush()
    expect(harness.client.admin.users.create).toHaveBeenCalledWith(expect.objectContaining({
      role: 'member', mailAccess: 'all',
    }))
  })

  it('ignores scope responses from an earlier account form opened during a load', async () => {
    const pendingAssignments = [deferred<string[]>(), deferred<string[]>(), deferred<string[]>()]
    vi.mocked(harness.client.admin.scopes.assignments)
      .mockImplementationOnce(() => pendingAssignments[0]!.promise)
      .mockImplementationOnce(() => pendingAssignments[1]!.promise)
      .mockImplementationOnce(() => pendingAssignments[2]!.promise)
    mount(); await flush()
    editButton('Member')!.props.onClick()
    editButton('Second member')!.props.onClick()
    await flush()
    expect(textOf()).toContain('Beta Projects')
    expect(textOf()).toContain('Alpha Team')
    expect(scopeCheckbox('scope-two')!.props.checked).toBe(true)
    pendingAssignments[0]!.resolve([existingUser.id])
    pendingAssignments[1]!.resolve([])
    pendingAssignments[2]!.resolve([])
    await flush()
    expect(scopeCheckbox('scope-two')!.props.checked).toBe(true)
    expect(scopeCheckbox('scope-one')!.props.checked).toBe(false)
    expect(scopeCheckbox('scope-three')!.props.checked).toBe(false)
  })
})
