import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { compileScript, compileTemplate, parse } from '@vue/compiler-sfc'
import { createRenderer, defineComponent, h, nextTick, ssrContextKey } from 'vue'
import * as VueRuntime from 'vue'
import type { SandpostEmailPreviewBridge } from '@/api/desktop'
import { i18n } from '@/i18n'

const theme = vi.hoisted(() => ({ isDark: null as import('vue').Ref<boolean> | null }))

vi.mock('@/composables/useTheme', async () => {
  const { ref } = await vi.importActual<typeof import('vue')>('vue')
  const isDark = ref(false)
  theme.isDark = isDark
  return {
    /**
     * Expose the controllable appearance state used by the preview test.
     */
    useTheme: () => ({ isDark })
  }
})

interface HostNode {
  type: string
  props: Record<string, unknown>
  children: HostNode[]
  parent: HostNode | null
  text?: string
  style?: Record<string, string>
  clientHeight: number

  /**
   * Provide the scrolling operation expected by the preview without a browser layout engine.
   */
  scrollIntoView: () => void

  /**
   * Return the geometry supplied by the host-node stub for preview layout calculations.
   */
  getBoundingClientRect: () => DOMRect

  /**
   * Return the client rectangles supplied by the host-node stub for visibility checks.
   */
  getClientRects: () => DOMRect[]
}

vi.mock('@/components/ui/button', () => ({
  Button: defineComponent({
    /**
     * Create the component stand-in render function used by the custom Vue test renderer.
     */
    setup: (_, { attrs, slots }) => () => h('button', attrs, slots.default?.()),
  }),
}))
vi.mock('@lucide/vue', () => ({
  CodeXmlIcon: defineComponent(() => () => h('span')),
  GripHorizontalIcon: defineComponent(() => () => h('span')),
  Loader2Icon: defineComponent(() => () => h('span')),
}))
vi.mock('@/components/ui/spinner', () => ({
  Spinner: defineComponent({
    /**
     * Create the component stand-in render function used by the custom Vue test renderer.
     */
    setup: (_, { attrs }) => () => h('span', attrs)
  }),
}))
vi.mock('@/components/ui/skeleton', () => ({
  Skeleton: defineComponent({
    /**
     * Create the component stand-in render function used by the custom Vue test renderer.
     */
    setup: (_, { attrs }) => () => h('span', attrs)
  }),
}))

const renderer = createRenderer<HostNode, HostNode>({
  /**
   * Create an element host node with the state required by the custom Vue test renderer.
   */
  createElement: (type) => ({
    type, props: {}, children: [], parent: null, clientHeight: 420,
    style: {},

    /**
     * Provide the scrolling operation expected by the preview without a browser layout engine.
     */
    scrollIntoView: () => { },

    /**
     * Return the geometry supplied by the host-node stub for preview layout calculations.
     */
    getBoundingClientRect: () => ({
      x: 0, y: 0, left: 0, top: 0, right: 500, bottom: 420, width: 500, height: 420,

      /**
       * Provide the JSON serialization stub required by the test rectangle.
       */
      toJSON: () => ({})
    }) as DOMRect,

    /**
     * Return the client rectangles supplied by the host-node stub for visibility checks.
     */
    getClientRects: () => [{} as DOMRect],
  }),

  /**
   * Create a text host node for the custom Vue test renderer.
   */
  createText: (text) => ({
    type: '#text', text, props: {}, children: [], parent: null, clientHeight: 0,

    /**
     * Provide the scrolling operation expected by the preview without a browser layout engine.
     */
    scrollIntoView: () => { },

    /**
     * Return the geometry supplied by the host-node stub for preview layout calculations.
     */
    getBoundingClientRect: () => ({} as DOMRect),

    /**
     * Return the client rectangles supplied by the host-node stub for visibility checks.
     */
    getClientRects: () => []
  }),

  /**
   * Create a comment host node for the custom Vue test renderer.
   */
  createComment: (text) => ({
    type: '#comment', text, props: {}, children: [], parent: null, clientHeight: 0,

    /**
     * Provide the scrolling operation expected by the preview without a browser layout engine.
     */
    scrollIntoView: () => { },

    /**
     * Return the geometry supplied by the host-node stub for preview layout calculations.
     */
    getBoundingClientRect: () => ({} as DOMRect),

    /**
     * Return the client rectangles supplied by the host-node stub for visibility checks.
     */
    getClientRects: () => []
  }),

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
  patchProp: (node, key, _previous, next) => { node.props[key] = next },

  /**
   * Attach a host node to its test parent at the requested anchor and record its parent.
   */
  insert: (node, parent, anchor) => {
    if (node.parent) node.parent.children.splice(node.parent.children.indexOf(node), 1)
    const index = anchor ? parent.children.indexOf(anchor) : -1
    if (index < 0) parent.children.push(node)
    else parent.children.splice(index, 0, node)
    node.parent = parent
  },

  /**
   * Detach the host node from its recorded test parent.
   */
  remove: (node) => {
    if (node.parent) node.parent.children.splice(node.parent.children.indexOf(node), 1)
    node.parent = null
  },

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
    const node: HostNode = {
      type: '#static', text: content, props: {}, children: [], parent: null, clientHeight: 0,

      /**
       * Provide the scrolling operation expected by the preview without a browser layout engine.
       */
      scrollIntoView: () => { },

      /**
       * Return the geometry supplied by the host-node stub for preview layout calculations.
       */
      getBoundingClientRect: () => ({} as DOMRect),

      /**
       * Return the client rectangles supplied by the host-node stub for visibility checks.
       */
      getClientRects: () => []
    }
    const index = anchor ? parent.children.indexOf(anchor) : -1
    if (index < 0) parent.children.push(node)
    else parent.children.splice(index, 0, node)
    node.parent = parent
    return [node, node]
  },
})

const componentSource = await import('./HtmlMessagePreview.vue?raw')
const descriptor = parse(componentSource.default, { filename: 'HtmlMessagePreview.vue' }).descriptor
const bindings = compileScript(descriptor, { id: 'html-message-preview-test' }).bindings
const compiledTemplate = compileTemplate({
  id: 'html-message-preview-test', filename: 'HtmlMessagePreview.vue', source: descriptor.template!.content,
  compilerOptions: { bindingMetadata: bindings },
})
const render = new Function('VueRuntime', compiledTemplate.code
  .replace(/^import \{([\s\S]*?)\} from "vue"\s*;?\s*/m, (_line, names: string) =>
    `const {${names.replace(/\s+as\s+/g, ': ')}} = VueRuntime;`)
  .replace('export function render', 'return function render'))(VueRuntime) as (this: unknown) => unknown
const { default: HtmlMessagePreview } = await import('./HtmlMessagePreview.vue')
  ; (HtmlMessagePreview as typeof HtmlMessagePreview & { render: typeof render }).render = render

let root: HostNode
let frames: Map<number, FrameRequestCallback>
let cancelledFrames: number[]
let observer: { observe: ReturnType<typeof vi.fn>; disconnect: ReturnType<typeof vi.fn> }
let bridge: SandpostEmailPreviewBridge

/**
 * Return every descendant matching a host tag.
 */
function findNodes(node: HostNode, type: string): HostNode[] {
  return [...(node.type === type ? [node] : []), ...node.children.flatMap((child) => findNodes(child, type))]
}

/**
 * Set browser stubs and an optional native preview bridge for component lifecycle tests.
 */
function installEnvironment(native = false): void {
  frames = new Map()
  cancelledFrames = []
  let nextFrame = 0
  observer = { observe: vi.fn(), disconnect: vi.fn() }
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => { const id = ++nextFrame; frames.set(id, callback); return id })
  vi.stubGlobal('cancelAnimationFrame', (id: number) => cancelledFrames.push(id))
  vi.stubGlobal('ResizeObserver', class { observe = observer.observe; disconnect = observer.disconnect })
  vi.stubGlobal('crypto', {
    /**
     * Return a deterministic identifier for the preview lifecycle test environment.
     */
    randomUUID: () => `uuid-${nextFrame}`
  })
  vi.stubGlobal('window', { innerWidth: 1000, innerHeight: 1000, addEventListener: vi.fn(), removeEventListener: vi.fn(), sandpost: native ? { runtime: 'electron', preview: bridge } : undefined })
  vi.stubGlobal('document', { hidden: false, querySelectorAll: vi.fn(() => []), addEventListener: vi.fn(), removeEventListener: vi.fn() })
  vi.stubGlobal('getComputedStyle', () => ({ visibility: 'visible', display: 'block', overflowX: 'visible', overflowY: 'visible' }))
}

/**
 * Flush Vue updates and promise continuations after bridge activity.
 */
async function flush(): Promise<void> {
  for (let index = 0; index < 4; index += 1) { await Promise.resolve(); await nextTick() }
}

beforeEach(() => {
  theme.isDark!.value = false
  root = {
    type: 'root', props: {}, children: [], parent: null, clientHeight: 0,

    /**
     * Provide the scrolling operation expected by the preview without a browser layout engine.
     */
    scrollIntoView: () => { },

    /**
     * Return the geometry supplied by the host-node stub for preview layout calculations.
     */
    getBoundingClientRect: () => ({} as DOMRect),

    /**
     * Return the client rectangles supplied by the host-node stub for visibility checks.
     */
    getClientRects: () => []
  }
  bridge = {
    show: vi.fn(async () => undefined), layout: vi.fn(async () => undefined), inspect: vi.fn(async () => undefined), release: vi.fn(async () => undefined),
    subscribe: vi.fn(() => vi.fn()),
  }
  installEnvironment()
})

afterEach(() => vi.unstubAllGlobals())

/**
 * Mount the preview with the shared English i18n plugin.
 */
function mountPreview(props: { document: string; messageIdentifier: string }) {
  i18n.global.locale.value = 'en'
  const app = renderer.createApp(HtmlMessagePreview, props)
  app.use(i18n)
  app.provide(ssrContextKey, { modules: new Set() })
  app.mount(root)
  return app
}

describe('HtmlMessagePreview', () => {
  it('keeps browser previews in the iframe without inspector UI, native API calls, or animation frames', async () => {
    const app = mountPreview({ document: '<p>body</p>', messageIdentifier: 'message-1' })
    await flush()
    expect(findNodes(root, 'iframe')).toHaveLength(1)
    expect(findNodes(root, 'button')).toHaveLength(0)
    expect(bridge.show).not.toHaveBeenCalled()
    expect(frames.size).toBe(0)
    app.unmount()
  })

  it('shows the actual document natively, toggles inspector state, handles close notifications, and releases on unmount', async () => {
    installEnvironment(true)
    const app = mountPreview({ document: '<p>actual email</p>', messageIdentifier: 'message-1' })
    await flush()
    expect(bridge.show).toHaveBeenCalledWith(expect.objectContaining({ id: expect.stringContaining('message-1'), html: '<p>actual email</p>' }))

    const inspect = findNodes(root, 'button')[0]!
      ; (findNodes(root, 'button')[0]!.props.onClick as () => void)()
    await flush()
    expect(bridge.inspect).toHaveBeenLastCalledWith(expect.objectContaining({ open: true }))
    expect(findNodes(root, 'div').some((node) => node.props['aria-label'] === 'Native email DevTools')).toBe(true)

      ; (inspect.props.onClick as () => void)()
    await flush()
    expect(bridge.inspect).toHaveBeenLastCalledWith(expect.objectContaining({ open: false }))

    const listener = vi.mocked(bridge.subscribe).mock.calls[0]![0]
    listener({ id: vi.mocked(bridge.show).mock.calls[0]![0].id, open: true })
    listener({ id: vi.mocked(bridge.show).mock.calls[0]![0].id, open: false })
    expect(findNodes(root, 'div').some((node) => node.props['aria-label'] === 'Native email DevTools')).toBe(false)
    app.unmount()
    expect(bridge.release).toHaveBeenCalledWith(vi.mocked(bridge.show).mock.calls[0]![0].id)
    expect(observer.disconnect).toHaveBeenCalled()
    expect(cancelledFrames.length).toBeGreaterThan(0)
  })

  it('shows an accessible loading state until the current native inspector is ready', async () => {
    installEnvironment(true)
    const app = mountPreview({ document: '<p>email</p>', messageIdentifier: 'loading' })
    await flush()

    const identifier = vi.mocked(bridge.show).mock.calls[0]![0].id
    const listener = vi.mocked(bridge.subscribe).mock.calls[0]![0]
      ; (findNodes(root, 'button')[0]!.props.onClick as () => void)()
    await nextTick()

    /**
     * Find the host element representing the native email inspector.
     */
    const inspector = () => findNodes(root, 'div').find((node) => node.props['aria-label'] === 'Native email DevTools')

    /**
     * Find the status element shown while the native inspector is opening.
     */
    const loader = () => findNodes(root, 'div').find((node) => node.props.role === 'status' && node.props['aria-label'] === 'Opening inspector')
    expect(loader()).toBeDefined()
    expect(inspector()?.props['aria-busy']).toBe(true)
    expect(findNodes(root, 'p').some((node) => node.text?.includes('Scroll the preview fully into view'))).toBe(false)

    listener({ id: 'stale-inspector', open: true, ready: true })
    await flush()
    expect(loader()).toBeDefined()
    expect(inspector()?.props['aria-busy']).toBe(true)

    listener({ id: identifier, open: true, ready: true })
    await flush()
      ;[...frames.values()].at(-1)?.(0)
    await flush()
    expect(loader()).toBeUndefined()
    expect(inspector()?.props['aria-busy']).toBe(false)
    expect(findNodes(root, 'p').some((node) => node.text?.includes('Scroll the preview fully into view'))).toBe(false)

    const preview = findNodes(root, 'div').find((node) => String(node.props.class).includes('relative min-h-0'))!
    preview.getClientRects = () => []
      ;[...frames.values()].at(-1)?.(0)
    await flush()
    expect(findNodes(root, 'p').some((node) => node.text?.includes('Scroll the preview fully into view'))).toBe(true)

    listener({ id: identifier, open: false, ready: true })
    await flush()
      ; (findNodes(root, 'button')[0]!.props.onClick as () => void)()
    await nextTick()
    expect(loader()).toBeDefined()
    expect(inspector()?.props['aria-busy']).toBe(true)
    app.unmount()
  })

  it.each([false, true])('updates native layout when the shared theme changes from %s', async (initialDark) => {
    theme.isDark!.value = initialDark
    installEnvironment(true)
    const app = mountPreview({ document: '<p>email</p>', messageIdentifier: 'theme' })
    await flush()

      ;[...frames.values()].at(-1)?.(0)
    await flush()
    expect(bridge.layout).toHaveBeenLastCalledWith(expect.objectContaining({ theme: initialDark ? 'dark' : 'light' }))

      ; (findNodes(root, 'button')[0]!.props.onClick as () => void)()
    await flush()
    expect(bridge.inspect).toHaveBeenLastCalledWith(expect.objectContaining({ open: true }))
    const inspectCallsWhileOpen = vi.mocked(bridge.inspect).mock.calls.length
    const target = vi.mocked(bridge.show).mock.calls[0]![0]

    theme.isDark!.value = !initialDark
    await flush()
      ;[...frames.values()].at(-1)?.(0)
    await flush()
    expect(bridge.layout).toHaveBeenLastCalledWith(expect.objectContaining({ theme: initialDark ? 'light' : 'dark' }))
    expect(bridge.show).toHaveBeenCalledTimes(1)
    expect(vi.mocked(bridge.show).mock.calls[0]![0].id).toBe(target.id)
    expect(bridge.inspect).toHaveBeenCalledTimes(inspectCallsWhileOpen)

    const updatedLayoutCalls = vi.mocked(bridge.layout).mock.calls.length
      ;[...frames.values()].at(-1)?.(0)
    await flush()
    expect(bridge.layout).toHaveBeenCalledTimes(updatedLayoutCalls)
    app.unmount()
  })

  it('ignores stale show completions and falls back to the iframe on a current native error', async () => {
    let resolveOld!: () => void
    vi.mocked(bridge.show).mockImplementationOnce(() => new Promise<void>((resolve) => { resolveOld = resolve }))
    installEnvironment(true)
    const app = mountPreview({ document: '<p>first</p>', messageIdentifier: 'first' })
    await nextTick()
    const component = app._instance!
    component.props.document = '<p>second</p>'
    component.props.messageIdentifier = 'second'
    await flush()
    resolveOld()
    await flush()
    expect(findNodes(root, 'button')[0]!.props.disabled).toBeFalsy()
    expect(vi.mocked(bridge.show).mock.calls.map(([input]) => input.html)).toEqual(['<p>first</p>', '<p>second</p>', '<p>second</p>'])

    vi.mocked(bridge.layout).mockRejectedValueOnce(new Error('native view failed'))
    const frame = findNodes(root, 'iframe')[0]!
    const runFrame = [...frames.values()][0]
    runFrame?.(0)
    await flush()
    expect(frame.props.style).toMatchObject({ visibility: 'visible' })
    expect(findNodes(root, 'button')[0]!.props.disabled).toBe(true)
    expect(findNodes(root, 'p').some((node) => node.props.role === 'status')).toBe(true)
    app.unmount()
  })

  it('finishes a divider drag across the fallback iframe and restores native geometry', async () => {
    installEnvironment(true)
    const app = mountPreview({ document: '<p>email</p>', messageIdentifier: 'drag' })
    await flush()
      ; (findNodes(root, 'button')[0]!.props.onClick as () => void)()
    await flush()
    const divider = findNodes(root, 'div').find((node) => node.props.role === 'separator')!
    const capture = vi.fn()
      ; (divider.props.onPointerdown as (event: unknown) => void)({ button: 0, pointerId: 1, currentTarget: { setPointerCapture: capture }, preventDefault: vi.fn() })
    await flush()
    expect(capture).toHaveBeenCalledWith(1)
    expect(findNodes(root, 'iframe')[0]!.props.style).toMatchObject({ pointerEvents: 'none' })
    expect(bridge.layout).toHaveBeenLastCalledWith(expect.objectContaining({ preview: null, devtools: null }))
    const listeners = vi.mocked(window.addEventListener).mock.calls
    const move = listeners.find(([name]) => name === 'pointermove')![1] as (event: unknown) => void
    const up = listeners.find(([name]) => name === 'pointerup')![1] as () => void
    move({ clientY: 200 })
    await flush()
    expect(divider.props['aria-valuenow']).toBe(220)
    up()
    await flush()
    expect(findNodes(root, 'iframe')[0]!.props.style).toMatchObject({ pointerEvents: undefined })
      ;[...frames.values()][0]?.(0)
    await flush()
    expect(bridge.layout).toHaveBeenLastCalledWith(expect.objectContaining({ preview: expect.objectContaining({ width: 500 }) }))
    app.unmount()
    expect(window.removeEventListener).toHaveBeenCalledWith('pointerup', up)
  })

  it('keeps a close request when the earlier native open acknowledgement arrives late', async () => {
    installEnvironment(true)
    const app = mountPreview({ document: '<p>email</p>', messageIdentifier: 'toggle' })
    await flush()
    const click = findNodes(root, 'button')[0]!.props.onClick as () => void
    click()
    await flush()
    expect(bridge.inspect).toHaveBeenLastCalledWith(expect.objectContaining({ open: true }))
    let finishLayout!: () => void
    vi.mocked(bridge.layout).mockImplementationOnce(() => new Promise<void>((resolve) => { finishLayout = resolve }))
    click()
    await flush()
    const identifier = vi.mocked(bridge.show).mock.calls[0]![0].id
    vi.mocked(bridge.subscribe).mock.calls[0]![0]({ id: identifier, open: true })
    finishLayout()
    await flush()
    expect(bridge.inspect).toHaveBeenLastCalledWith({ id: identifier, open: false })
    app.unmount()
  })

  it('discards a pending open operation after a newer toggle closes the inspector', async () => {
    installEnvironment(true)
    const app = mountPreview({ document: '<p>email</p>', messageIdentifier: 'rapid' })
    await flush()
    let finishLayout!: () => void
    vi.mocked(bridge.layout).mockImplementationOnce(() => new Promise<void>((resolve) => { finishLayout = resolve }))
    const click = findNodes(root, 'button')[0]!.props.onClick as () => void
    click()
    await flush()
    click()
    await flush()
    finishLayout()
    await flush()
    expect(bridge.inspect).toHaveBeenCalledTimes(1)
    expect(bridge.inspect).toHaveBeenCalledWith(expect.objectContaining({ open: false }))
    app.unmount()
  })
})
