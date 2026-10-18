import { describe, expect, it, vi } from 'vitest'
import { compileScript, compileTemplate, parse } from '@vue/compiler-sfc'
import { createRenderer, defineComponent, h, nextTick, ref, ssrContextKey, type Component } from 'vue'
import * as VueRuntime from 'vue'
import { FOLDER_ICON_OPTIONS, folderIcon } from './folderIcons'
import FolderIconPicker from './FolderIconPicker.vue'
import folderIconPickerSource from './FolderIconPicker.vue?raw'

interface TestNode {
  type: string
  props: Record<string, unknown>
  children: TestNode[]
  parent: TestNode | null
  text?: string
}

const pickerDescriptor = parse(folderIconPickerSource, { filename: 'FolderIconPicker.vue' }).descriptor
const pickerBindings = compileScript(pickerDescriptor, { id: 'folder-icon-picker-test' }).bindings
const pickerTemplate = compileTemplate({
  id: 'folder-icon-picker-test',
  filename: 'FolderIconPicker.vue',
  source: pickerDescriptor.template!.content,
  compilerOptions: { bindingMetadata: pickerBindings },
})
const pickerRender = new Function('VueRuntime', pickerTemplate.code
  .replace(/^import \{([\s\S]*?)\} from "vue"\s*;?\s*/m, (_statement, names: string) =>
    `const {${names.replace(/\s+as\s+/g, ': ')}} = VueRuntime;`)
  .replace('export function render', 'return function render'))(VueRuntime) as (this: unknown) => unknown
;(FolderIconPicker as typeof FolderIconPicker & { render: typeof pickerRender }).render = pickerRender

const previewDefaultIcon = defineComponent({
  setup: () => () => h('span', { 'data-default-icon': true }),
})

/**
 * Create the in-memory renderer used to exercise picker controls without a browser.
 */
function createTestRenderer() {
  return createRenderer<TestNode, TestNode>({
    createElement: type => ({ type, props: {}, children: [], parent: null }),
    createText: text => ({ type: '#text', text, props: {}, children: [], parent: null }),
    createComment: text => ({ type: '#comment', text, props: {}, children: [], parent: null }),
    setText: (node, text) => { node.text = text },
    setElementText: (node, text) => { node.children = []; node.text = text },
    patchProp: (node, key, _previous, value) => { node.props[key] = value },
    insert: (node, parent, anchor) => {
      if (node.parent) node.parent.children.splice(node.parent.children.indexOf(node), 1)
      node.parent = parent
      const index = anchor ? parent.children.indexOf(anchor) : -1
      parent.children.splice(index < 0 ? parent.children.length : index, 0, node)
    },
    remove: node => {
      if (!node.parent) return
      node.parent.children.splice(node.parent.children.indexOf(node), 1)
      node.parent = null
    },
    parentNode: node => node.parent,
    nextSibling: node => {
      if (!node.parent) return null
      return node.parent.children[node.parent.children.indexOf(node) + 1] ?? null
    },
  })
}

/**
 * Find the first rendered host element matching a predicate.
 */
function findNode(node: TestNode, predicate: (candidate: TestNode) => boolean): TestNode | undefined {
  if (predicate(node)) return node
  for (const child of node.children) {
    const result = findNode(child, predicate)
    if (result) return result
  }
  return undefined
}

/**
 * Return all rendered host elements matching a predicate.
 */
function findNodes(node: TestNode, predicate: (candidate: TestNode) => boolean): TestNode[] {
  return [
    ...(predicate(node) ? [node] : []),
    ...node.children.flatMap(child => findNodes(child, predicate)),
  ]
}

/**
 * Render the picker with primitive stand-ins and an observable model value.
 */
function mountPicker(initialIcon: string | null = null, defaultIcon: Component = previewDefaultIcon, disabled = false) {
  const selectedIcon = ref(initialIcon)
  const root: TestNode = { type: 'root', props: {}, children: [], parent: null }
  const renderer = createTestRenderer()
  const harness = defineComponent({
    setup() {
      return () => h(FolderIconPicker, {
        modelValue: selectedIcon.value,
        'onUpdate:modelValue': (value: string | null) => { selectedIcon.value = value },
        defaultIcon,
        disabled,
      })
    },
  })
  const app = renderer.createApp(harness)
  app.provide(ssrContextKey, { modules: new Set<string>() })
  app.mount(root)
  return { app, root, selected: () => selectedIcon.value }
}

/**
 * Invoke a rendered control handler with a minimal event value.
 */
function click(node: TestNode): void {
  const handler = node.props.onClick as ((event: { preventDefault(): void }) => void) | undefined
  handler?.({ preventDefault() {} })
}

vi.mock('vue-i18n', () => ({
  useI18n: () => ({ t: (key: string, values?: Record<string, string>) => values?.icon ? `${key}:${values.icon}` : key }),
}))

vi.mock('@/components/ui/button', () => ({
  Button: defineComponent({
    inheritAttrs: false,
    setup: (_, { attrs, slots }) => () => h('button', attrs, slots.default?.()),
  }),
}))

vi.mock('@/components/ui/input', () => ({
  Input: defineComponent({
    props: { modelValue: String },
    emits: ['update:modelValue'],
    setup: (props, { attrs, emit }) => () => h('input', {
      ...attrs,
      value: props.modelValue,
      onInput: (event: { target: { value: string } }) => emit('update:modelValue', event.target.value),
    }),
  }),
}))

vi.mock('@/components/ui/popover', () => ({
  Popover: defineComponent({ setup: (_, { slots }) => () => h('div', slots.default?.()) }),
  PopoverContent: defineComponent({ setup: (_, { slots }) => () => h('section', slots.default?.()) }),
  PopoverTrigger: defineComponent({ setup: (_, { slots }) => () => h('div', slots.default?.()) }),
}))

describe('folder icon catalog', () => {
  /**
   * Keep catalog identifiers unique, kebab-case, and backed by a Vue component.
   */
  it('provides diverse stable IDs and a component for each option', () => {
    expect(FOLDER_ICON_OPTIONS.length).toBeGreaterThanOrEqual(142)
    expect(new Set(FOLDER_ICON_OPTIONS.map(option => option.id)).size).toBe(FOLDER_ICON_OPTIONS.length)
    expect(FOLDER_ICON_OPTIONS.every(option => /^[a-z][a-z0-9]*(?:-[a-z0-9]+)*$/.test(option.id) && option.component)).toBe(true)
    expect(FOLDER_ICON_OPTIONS.every(option => folderIcon(option.id, 'folder') === option.component)).toBe(true)
  })

  /**
   * Resolve explicit choices and defaults while keeping unsupported IDs safe.
   */
  it('resolves custom icons, kind defaults, and unknown IDs without changing the stored value', () => {
    expect(folderIcon('mail', 'folder')).toBe(FOLDER_ICON_OPTIONS.find(option => option.id === 'mail')?.component)
    expect(folderIcon(null, 'inbox')).toBe(folderIcon('inbox', 'folder'))
    expect(folderIcon('missing-icon', 'smart')).toBe(folderIcon(null, 'smart'))
    expect(folderIcon('missing-icon', 'address')).toBe(folderIcon(null, 'address'))
  })
})

describe('FolderIconPicker', () => {
  /**
   * Verify explicit selection replaces an unknown ID and reset restores the default value.
   */
  it('selects a real option and resets an override to the default', async () => {
    const { app, root, selected } = mountPicker('missing-icon')
    expect(selected()).toBe('missing-icon')
    expect(findNode(root, node => node.props['data-default-icon'] === true)).toBeDefined()
    const mailOption = findNode(root, node => node.type === 'button' && node.props['aria-label'] === 'mail')
    expect(mailOption?.props['aria-pressed']).toBe(false)
    click(mailOption!)
    await nextTick()
    expect(selected()).toBe('mail')
    expect(findNode(root, node => node.type === 'button' && node.props['aria-label'] === 'mail')?.props.variant).toBe('secondary')

    const reset = findNode(root, node => node.type === 'button' && node.children.some(child => child.text?.includes('mail.icon.default')))
    click(reset!)
    await nextTick()
    expect(selected()).toBeNull()
    app.unmount()
  })

  /**
   * Verify typed search leaves only the matching icon in the accessible grid.
   */
  it('filters icon options from the searchable grid', async () => {
    const { app, root } = mountPicker()
    const search = findNode(root, node => node.type === 'input')
    ;(search!.props.onInput as (event: { target: { value: string } }) => void)({ target: { value: 'flask conical' } })
    await nextTick()
    await nextTick()
    expect(search!.props.value).toBe('flask conical')
    const labels = findNodes(root, node => node.type === 'button').map(node => node.props['aria-label'])
    expect(labels).toContain('flask conical')
    expect(labels).not.toContain('folder search')
    ;(search!.props.onInput as (event: { target: { value: string } }) => void)({ target: { value: 'flask-conical' } })
    await nextTick()
    await nextTick()
    expect(findNodes(root, node => node.type === 'button').map(node => node.props['aria-label'])).toContain('flask conical')
    app.unmount()
  })

  /**
   * Disable the search, icon choices, reset action, and trigger together while saving.
   */
  it('disables every picker control when requested', () => {
    const { app, root } = mountPicker('mail', previewDefaultIcon, true)
    expect(findNodes(root, node => node.type === 'button').every(node => node.props.disabled === true)).toBe(true)
    expect(findNode(root, node => node.type === 'input')?.props.disabled).toBe(true)
    app.unmount()
  })
})
