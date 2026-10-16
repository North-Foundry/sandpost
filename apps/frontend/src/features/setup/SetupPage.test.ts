import * as VueRuntime from 'vue'
import { createRenderer, nextTick, ssrContextKey } from 'vue'
import { createI18n } from 'vue-i18n'
import { compileScript, compileTemplate, parse } from '@vue/compiler-sfc'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import englishAuth from '@/i18n/en/auth'

const i18n = createI18n({ legacy: false, locale: 'en', fallbackLocale: 'en', flatJson: true, messages: { en: englishAuth } })

const harness = vi.hoisted(() => ({
  client: {
    instanceStatus: vi.fn(), authorizeSetup: vi.fn(), setup: vi.fn(),
  },
  router: { replace: vi.fn() },
}))

vi.mock('vue-router', () => ({
  /**
   * Expose the router double configured by the component test.
   */
  useRouter: () => harness.router
}))
vi.mock('@/runtime/client', () => ({
  /**
   * Expose the client double configured by the component test.
   */
  useSandpostClient: () => harness.client
}))

/*
 * Render the real form SFCs with small stand-ins for visual UI primitives.
 */
vi.mock('@/components/ui/alert', async () => {
  const { defineComponent, h } = await import('vue')

  /**
   * Create a visual component stand-in that forwards attributes and slots to the requested host tag.
   */
  const part = (tag: string) => defineComponent({
    /**
     * Create the component stand-in render function used by the custom Vue test renderer.
     */
    setup: (_, { attrs, slots }) => () => h(tag, attrs, slots.default?.())
  })
  return { Alert: part('div'), AlertDescription: part('div'), AlertTitle: part('h2') }
})
vi.mock('@/components/ui/button', async () => {
  const { defineComponent, h } = await import('vue')
  return {
    Button: defineComponent({
      /**
       * Create the component stand-in render function used by the custom Vue test renderer.
       */
      setup: (_, { attrs, slots }) => () => h('button', attrs, slots.default?.())
    })
  }
})
vi.mock('@/components/ui/card', async () => {
  const { defineComponent, h } = await import('vue')

  /**
   * Create a visual component stand-in that forwards attributes and slots to the requested host tag.
   */
  const part = (tag: string) => defineComponent({
    /**
     * Create the component stand-in render function used by the custom Vue test renderer.
     */
    setup: (_, { attrs, slots }) => () => h(tag, attrs, slots.default?.())
  })
  return { Card: part('section'), CardContent: part('div'), CardDescription: part('p'), CardFooter: part('div'), CardHeader: part('header'), CardTitle: part('div') }
})
vi.mock('@/components/ui/collapsible', async () => {
  const { defineComponent, h } = await import('vue')

  /**
   * Create a visual component stand-in that forwards attributes and slots to the requested host tag.
   */
  const part = (tag: string) => defineComponent({
    /**
     * Create the component stand-in render function used by the custom Vue test renderer.
     */
    setup: (_, { attrs, slots }) => () => h(tag, attrs, slots.default?.())
  })
  return { Collapsible: part('div'), CollapsibleContent: part('div'), CollapsibleTrigger: part('div') }
})
vi.mock('@/components/ui/field', async () => {
  const { defineComponent, h } = await import('vue')

  /**
   * Create a visual component stand-in that forwards attributes and slots to the requested host tag.
   */
  const part = (tag: string) => defineComponent({
    /**
     * Create the component stand-in render function used by the custom Vue test renderer.
     */
    setup: (_, { attrs, slots }) => () => h(tag, attrs, slots.default?.())
  })
  return { Field: part('div'), FieldError: part('p'), FieldGroup: part('div'), FieldLabel: part('label'), FieldDescription: part('p') }
})
vi.mock('@/components/ui/input', async () => {
  const { defineComponent, h } = await import('vue')
  return {
    Input: defineComponent({
      inheritAttrs: false, props: ['modelValue'], emits: ['update:modelValue'],

      /**
       * Create the component stand-in render function used by the custom Vue test renderer.
       */
      setup: (props, { attrs, emit }) => () => h('input', {
        ...attrs, value: props.modelValue,

        /**
         * Forward the input element value through the component stand-in model update.
         */
        onInput: (event: Event) => emit('update:modelValue', (event.target as HTMLInputElement).value),
      }),
    })
  }
})
vi.mock('@/components/ui/spinner', async () => {
  const { defineComponent, h } = await import('vue')
  return {
    Spinner: defineComponent({
      /**
       * Create the component stand-in render function used by the custom Vue test renderer.
       */
      setup: (_, { attrs }) => () => h('span', attrs)
    })
  }
})
vi.mock('@/layouts/AuthLayout.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return {
    default: defineComponent({
      /**
       * Create the component stand-in render function used by the custom Vue test renderer.
       */
      setup: (_, { slots }) => () => h('main', slots.default?.())
    })
  }
})

import SetupPage from './SetupPage.vue'
import pageSource from './SetupPage.vue?raw'
import SetupKeyForm from '@/features/setup/SetupKeyForm.vue'
import keyFormSource from '@/features/setup/SetupKeyForm.vue?raw'
import CreateOwnerForm from '@/features/setup/CreateOwnerForm.vue'
import ownerFormSource from '@/features/setup/CreateOwnerForm.vue?raw'

/**
 * Compile an SFC template for the node renderer used by this test suite.
 */
function attachTemplate(component: object, source: string, filename: string, id: string): void {
  const descriptor = parse(source, { filename }).descriptor
  const bindings = compileScript(descriptor, { id }).bindings
  const template = compileTemplate({ id, filename, source: descriptor.template!.content, compilerOptions: { bindingMetadata: bindings } })
  const render = new Function('VueRuntime', template.code
    .replace(/^import \{([\s\S]*?)\} from "vue"\s*;?\s*/m, (_statement, names: string) =>
      `const {${names.replace(/\s+as\s+/g, ': ')}} = VueRuntime;`)
    .replace('export function render', 'return function render'))(VueRuntime) as (this: unknown) => unknown
    ; (component as { render?: typeof render }).render = render
}

attachTemplate(SetupPage, pageSource, 'SetupPage.vue', 'setup-page-test')
attachTemplate(SetupKeyForm, keyFormSource, 'SetupKeyForm.vue', 'setup-key-form-test')
attachTemplate(CreateOwnerForm, ownerFormSource, 'CreateOwnerForm.vue', 'create-owner-form-test')

interface TestNode {
  type: string
  props: Record<string, any>
  children: TestNode[]
  parent: TestNode | null
  text?: string
}

const renderer = createRenderer<TestNode, TestNode>({
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
  setText: (node, text) => { node.text = text },

  /**
   * Replace an element host node text and clear its previous child nodes.
   */
  setElementText: (node, text) => { node.text = text; node.children = [] },

  /**
   * Store the updated component attribute on its host-node test double.
   */
  patchProp: (node, key, _previous, value) => { node.props[key] = value },

  /**
   * Attach a host node to its test parent at the requested anchor and record its parent.
   */
  insert: (node, parent, anchor) => {
    const index = anchor ? parent.children.indexOf(anchor) : -1
    if (index < 0) parent.children.push(node)
    else parent.children.splice(index, 0, node)
    node.parent = parent
  },

  /**
   * Detach the host node from its recorded test parent.
   */
  remove: (node) => { if (node.parent) parentRemove(node) },

  /**
   * Return the parent recorded for this host node by the custom Vue test renderer.
   */
  parentNode: (node) => node.parent,

  /**
   * Return the next host node in the test parent, or null at the end of its children.
   */
  nextSibling: (node) => node.parent?.children[node.parent.children.indexOf(node) + 1] ?? null,

  /**
   * Insert a static-content host node and return its start and end nodes to Vue.
   */
  insertStaticContent: (content, parent, anchor) => {
    const node: TestNode = { type: '#static', text: content, props: {}, children: [], parent: null }
    const index = anchor ? parent.children.indexOf(anchor) : -1
    if (index < 0) parent.children.push(node)
    else parent.children.splice(index, 0, node)
    node.parent = parent
    return [node, node]
  },
})

/**
 * Remove a node from its parent during renderer unmount.
 */
function parentRemove(node: TestNode): void {
  const parent = node.parent!
  parent.children.splice(parent.children.indexOf(node), 1)
  node.parent = null
}

/**
 * Find the first rendered node matching a predicate.
 */
function findNode(nodes: TestNode[], predicate: (node: TestNode) => boolean): TestNode | undefined {
  for (const node of nodes) {
    if (predicate(node)) return node
    const match = findNode(node.children, predicate)
    if (match) return match
  }
}

/**
 * Return visible text from the custom renderer tree.
 */
function textContent(node: TestNode): string {
  return `${node.text ?? ''}${node.children.map(textContent).join('')}`
}

/**
 * Let mounted status requests and their Vue updates settle.
 */
async function settle(): Promise<void> {
  for (let index = 0; index < 5; index++) {
    await Promise.resolve()
    await nextTick()
  }
}

/**
 * Create a deferred promise for checking the visible pending state.
 */
function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (reason?: unknown) => void
  const promise = new Promise<T>((accept, fail) => { resolve = accept; reject = fail })
  return { promise, resolve, reject }
}

describe('setup page', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    i18n.global.locale.value = 'en'
    harness.client.instanceStatus.mockResolvedValue({ initialized: false, setupKeyRequired: true })
    harness.client.authorizeSetup.mockResolvedValue({ authorization: 'opaque-setup-token' })
    harness.client.setup.mockResolvedValue(undefined)
    harness.router.replace.mockResolvedValue(undefined)
    vi.stubGlobal('document', { documentElement: { dataset: {} }, getElementById: vi.fn(() => ({ focus: vi.fn() })) })
  })

  it('uses the English Vue I18n catalog by default', () => {
    expect(i18n.global.locale.value).toBe('en')
  })

  /**
   * Mount the page and expose its rendered root for assertions.
   */
  function mount() {
    const app = renderer.createApp(SetupPage)
    app.use(i18n)
    app.provide(ssrContextKey, { modules: new Set() })
    const container: TestNode = { type: 'root', props: {}, children: [], parent: null }
    app.mount(container)
    return { app, root: container }
  }

  /**
   * Send native input events through the real compiled form model.
   */
  async function input(root: TestNode, id: string, value: string, typeEachCharacter = false): Promise<void> {
    const node = findNode(root.children, (candidate) => candidate.props.id === id)
    expect(node, `input #${id} should be rendered`).toBeDefined()
    const values = typeEachCharacter ? Array.from({ length: value.length }, (_, index) => value.slice(0, index + 1)) : [value]
    for (const typedValue of values) {
      node!.props.onInput({ target: { value: typedValue } })
      await nextTick()
      expect(node!.props.value).toBe(typedValue)
    }
  }

  it('shows a neutral loading state before status, then only the permitted form', async () => {
    const status = deferred<{ initialized: boolean; setupKeyRequired: boolean }>()
    harness.client.instanceStatus.mockReturnValueOnce(status.promise)
    const { app, root } = mount()
    expect(textContent(root)).toContain('Checking this instance…')
    expect(findNode(root.children, (node) => node.props.id === 'setup-key')).toBeUndefined()
    expect(findNode(root.children, (node) => node.props.id === 'owner-name')).toBeUndefined()
    status.resolve({ initialized: false, setupKeyRequired: false })
    await settle()
    expect(findNode(root.children, (node) => node.props.id === 'owner-name')).toBeDefined()
    expect(findNode(root.children, (node) => node.props.id === 'setup-key')).toBeUndefined()
    expect(harness.client.authorizeSetup).not.toHaveBeenCalled()
    app.unmount()
  })

  it('fails closed when the status omits setupKeyRequired', async () => {
    harness.client.instanceStatus.mockResolvedValueOnce({ initialized: false } as never)
    const { app, root } = mount()
    await settle()
    expect(findNode(root.children, (node) => node.props.id === 'setup-key')).toBeDefined()
    expect(findNode(root.children, (node) => node.props.id === 'owner-name')).toBeUndefined()
    app.unmount()
  })

  it('exchanges a typed and pasted setup key for an opaque token, creates the owner, then routes to login', async () => {
    const { app, root } = mount()
    await settle()
    await input(root, 'setup-key', '  mock key copied  ', true)
    const keyForm = findNode(root.children, (node) => node.type === 'form')!
    await keyForm.props.onSubmit({ preventDefault: vi.fn() })
    await settle()
    expect(harness.client.authorizeSetup).toHaveBeenCalledWith({ setupKey: 'mock key copied' })
    expect(findNode(root.children, (node) => node.props.id === 'setup-key')).toBeUndefined()
    expect(findNode(root.children, (node) => node.props.id === 'owner-name')).toBeDefined()

    await input(root, 'owner-name', ' Owner ')
    await input(root, 'owner-email', ' owner@example.test ')
    await input(root, 'owner-password', 'correct horse battery')
    await input(root, 'owner-confirmation', 'correct horse battery')
    const ownerForm = findNode(root.children, (node) => node.type === 'form')!
    await ownerForm.props.onSubmit({ preventDefault: vi.fn() })
    await settle()
    expect(harness.client.setup).toHaveBeenCalledWith({
      bootstrapSecret: 'opaque-setup-token', name: 'Owner', email: 'owner@example.test', password: 'correct horse battery',
    })
    expect(harness.client.setup.mock.calls[0]![0].bootstrapSecret).not.toBe('mock key copied')
    expect(harness.router.replace).toHaveBeenCalledWith({ name: 'login' })
    app.unmount()
  })

  it('rejects a stale startup key inline and permits the current key to retry', async () => {
    harness.client.authorizeSetup.mockRejectedValueOnce(Object.assign(new Error('Invalid setup key.'), { code: 'authentication_failed' }))
    const { app, root } = mount()
    await settle()
    await input(root, 'setup-key', 'old-startup-key')
    await findNode(root.children, (node) => node.type === 'form')!.props.onSubmit({ preventDefault: vi.fn() })
    await settle()
    expect(textContent(root)).toContain('Invalid setup key.')
    expect(findNode(root.children, (node) => node.props['aria-invalid'] === true)).toBeDefined()
    await input(root, 'setup-key', 'current-startup-key')
    await findNode(root.children, (node) => node.type === 'form')!.props.onSubmit({ preventDefault: vi.fn() })
    await settle()
    expect(harness.client.authorizeSetup).toHaveBeenNthCalledWith(1, { setupKey: 'old-startup-key' })
    expect(harness.client.authorizeSetup).toHaveBeenNthCalledWith(2, { setupKey: 'current-startup-key' })
    expect(findNode(root.children, (node) => node.props.id === 'owner-name')).toBeDefined()
    app.unmount()
  })

  it('returns to the key form after expired authorization while retaining the owner name and email', async () => {
    harness.client.setup.mockRejectedValueOnce(Object.assign(new Error('expired'), { status: 401, code: 'authentication_failed' }))
    const { app, root } = mount()
    await settle()
    await input(root, 'setup-key', 'startup-key')
    await findNode(root.children, (node) => node.type === 'form')!.props.onSubmit({ preventDefault: vi.fn() })
    await settle()
    await input(root, 'owner-name', 'Owner')
    await input(root, 'owner-email', 'owner@example.test')
    await input(root, 'owner-password', 'correct horse battery')
    await input(root, 'owner-confirmation', 'correct horse battery')
    await findNode(root.children, (node) => node.type === 'form')!.props.onSubmit({ preventDefault: vi.fn() })
    await settle()
    expect(findNode(root.children, (node) => node.props.id === 'setup-key')).toBeDefined()
    expect(textContent(root)).toContain('Invalid setup key.')
    await input(root, 'setup-key', 'fresh-startup-key')
    harness.client.authorizeSetup.mockResolvedValueOnce({ authorization: 'fresh-opaque-token' })
    await findNode(root.children, (node) => node.type === 'form')!.props.onSubmit({ preventDefault: vi.fn() })
    await settle()
    expect(findNode(root.children, (node) => node.props.id === 'owner-name')?.props.value).toBe('Owner')
    expect(findNode(root.children, (node) => node.props.id === 'owner-email')?.props.value).toBe('owner@example.test')
    await input(root, 'owner-password', 'new correct horse battery')
    await input(root, 'owner-confirmation', 'new correct horse battery')
    await findNode(root.children, (node) => node.type === 'form')!.props.onSubmit({ preventDefault: vi.fn() })
    await settle()
    expect(harness.client.setup.mock.calls.at(-1)?.[0].bootstrapSecret).toBe('fresh-opaque-token')
    app.unmount()
  })

  it('redirects when setup is already initialized or loses a 409 race', async () => {
    harness.client.instanceStatus.mockResolvedValueOnce({ initialized: true, setupKeyRequired: false })
    const initialized = mount()
    await settle()
    expect(harness.router.replace).toHaveBeenCalledWith({ name: 'login' })
    initialized.app.unmount()

    vi.clearAllMocks()
    harness.client.instanceStatus.mockResolvedValue({ initialized: false, setupKeyRequired: true })
    harness.client.authorizeSetup.mockResolvedValue({ authorization: 'opaque' })
    harness.client.setup.mockRejectedValueOnce(Object.assign(new Error('already initialized'), { status: 409, code: 'already_initialized' }))
    const raced = mount()
    await settle()
    await input(raced.root, 'setup-key', 'startup-key')
    await findNode(raced.root.children, (node) => node.type === 'form')!.props.onSubmit({ preventDefault: vi.fn() })
    await settle()
    await input(raced.root, 'owner-name', 'Owner')
    await input(raced.root, 'owner-email', 'owner@example.test')
    await input(raced.root, 'owner-password', 'correct horse battery')
    await input(raced.root, 'owner-confirmation', 'correct horse battery')
    await findNode(raced.root.children, (node) => node.type === 'form')!.props.onSubmit({ preventDefault: vi.fn() })
    await settle()
    expect(harness.router.replace).toHaveBeenCalledWith({ name: 'login' })
    expect(findNode(raced.root.children, (node) => node.props.id === 'owner-name')).toBeUndefined()
    raced.app.unmount()
  })

  it('offers retry after unavailable status and fetches status again as the source of truth', async () => {
    harness.client.instanceStatus.mockRejectedValueOnce(new Error('offline'))
      .mockResolvedValueOnce({ initialized: false, setupKeyRequired: false })
    vi.stubGlobal('localStorage', { getItem: vi.fn(), setItem: vi.fn() })
    vi.stubGlobal('sessionStorage', { getItem: vi.fn(), setItem: vi.fn() })
    const { app, root } = mount()
    await settle()
    expect(textContent(root)).toContain('Cannot reach Sandpost')
    const retry = findNode(root.children, (node) => node.type === 'button')!
    expect(textContent(retry)).toContain('Try again')
    retry.props.onClick()
    await settle()
    expect(harness.client.instanceStatus).toHaveBeenCalledTimes(2)
    expect(findNode(root.children, (node) => node.props.id === 'owner-name')).toBeDefined()
    expect(harness.client.authorizeSetup).not.toHaveBeenCalled()
    expect(localStorage.getItem).not.toHaveBeenCalled()
    expect(sessionStorage.getItem).not.toHaveBeenCalled()
    app.unmount()
  })

  it.each([
    ['name', 'name must contain 1 to 100 printable characters', 'Enter a name with 1 to 100 printable characters.'],
    ['email', 'enter a valid email address', 'Enter a valid email address.'],
    ['password', 'password must contain 12 to 1024 bytes', 'Use a password between 12 and 1024 bytes.'],
  ] as const)('maps backend %s validation beside the field without exposing internals', async (field, detail, message) => {
    harness.client.instanceStatus.mockResolvedValue({ initialized: false, setupKeyRequired: false })
    harness.client.setup.mockRejectedValueOnce(new Error(`private backend detail: ${detail}`))
    const { app, root } = mount()
    await settle()
    await input(root, 'owner-name', 'Owner')
    await input(root, 'owner-email', 'owner@example.test')
    await input(root, 'owner-password', 'correct horse battery')
    await input(root, 'owner-confirmation', 'correct horse battery')
    await findNode(root.children, (node) => node.type === 'form')!.props.onSubmit({ preventDefault: vi.fn() })
    await settle()
    expect(textContent(root)).toContain(message)
    expect(textContent(root)).not.toContain('private backend detail')
    expect(findNode(root.children, (node) => node.props.id === `owner-${field}-error`)).toBeDefined()
    app.unmount()
  })

  it('checks password confirmation and prevents duplicate submissions while busy', async () => {
    const authorization = deferred<{ authorization: string }>()
    harness.client.authorizeSetup.mockReturnValueOnce(authorization.promise)
    const { app, root } = mount()
    await settle()
    await input(root, 'setup-key', 'startup-key')
    const form = findNode(root.children, (node) => node.type === 'form')!
    const pending = form.props.onSubmit({ preventDefault: vi.fn() })
    await nextTick()
    const submit = findNode(root.children, (node) => node.type === 'button' && node.props.type === 'submit')!
    expect(submit.props.disabled).toBe(true)
    form.props.onSubmit({ preventDefault: vi.fn() })
    expect(harness.client.authorizeSetup).toHaveBeenCalledOnce()
    authorization.resolve({ authorization: 'opaque' })
    await pending
    await settle()
    await input(root, 'owner-password', 'correct horse battery')
    await input(root, 'owner-confirmation', 'different password')
    await findNode(root.children, (node) => node.type === 'form')!.props.onSubmit({ preventDefault: vi.fn() })
    await settle()
    expect(textContent(root)).toContain('Passwords do not match.')
    expect(harness.client.setup).not.toHaveBeenCalled()
    app.unmount()
  })

  it('focuses the newly enabled owner input after authorization finishes', async () => {
    const authorization = deferred<{ authorization: string }>()
    harness.client.authorizeSetup.mockReturnValueOnce(authorization.promise)
    const { app, root } = mount()
    await settle()
    const focused: { id: string; disabled: boolean }[] = []
    vi.spyOn(document, 'getElementById').mockImplementation((id) => {
      const target = findNode(root.children, (node) => node.props.id === id)
      return target ? {
        /**
         * Model the focus operation required by the component in the test environment.
         */
        focus: () => focused.push({ id, disabled: Boolean(target.props.disabled) })
      } as unknown as HTMLElement : null
    })
    await input(root, 'setup-key', 'startup-key')
    const submit = findNode(root.children, (node) => node.type === 'form')!
    const pending = submit.props.onSubmit({ preventDefault: vi.fn() })
    await nextTick()
    expect(findNode(root.children, (node) => node.props.id === 'setup-key')?.props.disabled).toBe(true)
    expect(focused).toEqual([])

    authorization.resolve({ authorization: 'opaque' })
    await pending
    await settle()
    expect(focused).toEqual([{ id: 'owner-name', disabled: false }])
    app.unmount()
  })

  it('does not restore the owner form or route after pending authorization resolves on unmount', async () => {
    const authorization = deferred<{ authorization: string }>()
    harness.client.authorizeSetup.mockReturnValueOnce(authorization.promise)
    const { app, root } = mount()
    await settle()
    const lookup = vi.spyOn(document, 'getElementById')
    await input(root, 'setup-key', 'startup-key')
    const pending = findNode(root.children, (node) => node.type === 'form')!.props.onSubmit({ preventDefault: vi.fn() })
    await nextTick()
    lookup.mockClear()
    app.unmount()

    authorization.resolve({ authorization: 'late-token' })
    await pending
    await settle()
    expect(root.children).toEqual([])
    expect(lookup).not.toHaveBeenCalled()
    expect(harness.client.setup).not.toHaveBeenCalled()
    expect(harness.router.replace).not.toHaveBeenCalled()
  })
})
