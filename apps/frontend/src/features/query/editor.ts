/**
 * CodeMirror setup for the query editor: a stream language sharing the tokenizer's word classification,
 * grammar-aware completion, hover documentation, inline diagnostics, and a theme drawn from the application's
 * CSS tokens.
 */
import {
  autocompletion, closeBrackets, closeBracketsKeymap, acceptCompletion, insertBracket, pickedCompletion, startCompletion,
  type Completion, type CompletionContext, type CompletionResult,
} from '@codemirror/autocomplete'
import { defaultKeymap, history, historyKeymap } from '@codemirror/commands'
import { HighlightStyle, StreamLanguage, bracketMatching, syntaxHighlighting, type StringStream } from '@codemirror/language'
import { setDiagnostics, type Diagnostic } from '@codemirror/lint'
import { Compartment, EditorState, Prec } from '@codemirror/state'
import {
  EditorView, highlightActiveLine, highlightActiveLineGutter, hoverTooltip, keymap, lineNumbers, placeholder, tooltips,
} from '@codemirror/view'
import { tags } from '@lezer/highlight'
import { characterIndexForByteOffset } from '@/api/admin'
import type { QueryDiagnostic } from '@/api/contracts'
import { asSentence } from '@/utils/errors'
import { i18n } from '@/i18n'
import { completionContext, describeTokenAt, positionedTokens, suggestionsFor, type QuerySuggestion } from './completion'
import { QUERY_TOKEN_CLASSES } from './tokenClasses'
import { classifyQueryWord, type QueryTokenKind } from './vocabulary'

/**
 * Collection fields compare with ANY semantics, so `!=` rarely means "exclude".
 */
const ANY_INEQUALITY = /\b(?:envelope\.to|to|from|cc|content)(?:\.(?:address|domain))?\s*!=|\bheader\[[^\]]*\]\s*!=/gi
const CLOSING: Record<string, string> = { '[': ']', '(': ')', '"': '"' }
type StyledTokenKind = Exclude<QueryTokenKind, 'plain'>
const TOKEN_TAGS = {
  field: tags.propertyName,
  operator: tags.operator,
  keyword: tags.keyword,
  string: tags.string,
  number: tags.number,
  punctuation: tags.bracket,
} satisfies Record<StyledTokenKind, unknown>

/**
 * Report whether a query uses `!=` on a collection field.
 */
export function hasAnyInequality(source: string): boolean {
  return source.search(ANY_INEQUALITY) >= 0
}

/**
 * Turn the server's diagnostic and local warnings into editor diagnostics for one source text.
 */
export function queryDiagnostics(source: string, error: QueryDiagnostic | null): Diagnostic[] {
  const diagnostics: Diagnostic[] = []
  if (error) {
    const index = Math.min(characterIndexForByteOffset(source, error.position), source.length)
    /*
     * Unknown words share a token with neighboring whitespace, so the range is trimmed to the visible text.
     */
    const token = positionedTokens(source).find((candidate) => candidate.from <= index && index < candidate.to && candidate.text.trim())
    const from = token
      ? Math.max(index, token.from + token.text.length - token.text.trimStart().length)
      : index >= source.length ? Math.max(0, source.length - 1) : index
    const to = token ? Math.max(from + 1, token.to - (token.text.length - token.text.trimEnd().length)) : Math.min(source.length, from + 1)
    diagnostics.push({ from, to, severity: 'error', message: asSentence(error.message) })
  }
  for (const match of source.matchAll(ANY_INEQUALITY)) {
    const from = match.index + match[0].lastIndexOf('!=')
    diagnostics.push({
      from, to: from + 2, severity: 'warning',
      message: i18n.global.t('query.warning.diagnostic'),
    })
  }
  return diagnostics
}

/**
 * Read the rest of a string literal, which may continue on later lines.
 */
function readString(stream: StringStream, state: { inString: boolean }): StyledTokenKind {
  state.inString = true
  while (!stream.eol()) {
    const character = stream.next()
    if (character === '\\') stream.next()
    else if (character === '"') { state.inString = false; break }
  }
  return 'string'
}

/**
 * The query grammar as a CodeMirror language, classifying words like the shared tokenizer so the editor and
 * static highlighting color the same tokens.
 */
const queryLanguage = StreamLanguage.define<{ inString: boolean }>({
  name: 'sandpost-query',

  /**
   * Start tokenization outside a string literal for each fresh parser state.
   */
  startState: () => ({ inString: false }),

  /**
   * Classifies the next source token using the shared query vocabulary.
   */
  token(stream, state): StyledTokenKind | null {
    if (state.inString) return readString(stream, state)
    if (stream.eatSpace()) return null
    if (stream.eat('"')) return readString(stream, state)
    if (stream.match(/^(?:==|!=|>=|<=|<|>)/)) return 'operator'
    if (stream.match(/^-?\d+/)) return 'number'
    const word = stream.match(/^[A-Za-z_][A-Za-z0-9_.]*/) as RegExpMatchArray | null
    if (word) {
      const kind = classifyQueryWord(word[0])
      return kind === 'plain' ? null : kind
    }
    return '()[]'.includes(stream.next() ?? '') ? 'punctuation' : null
  },
  languageData: { closeBrackets: { brackets: ['(', '[', '"'] } },
  tokenTable: TOKEN_TAGS,
})

const queryHighlighting = HighlightStyle.define((Object.keys(TOKEN_TAGS) as StyledTokenKind[])
  .map((kind) => ({ tag: TOKEN_TAGS[kind], class: QUERY_TOKEN_CLASSES[kind] })))

/**
 * Convert a suggestion into a completion that inserts its text, opens its brackets, and may chain.
 */
function toCompletion(suggestion: QuerySuggestion, index: number): Completion {
  return {
    label: suggestion.label,
    /*
     * Equal-scoring matches keep the curated order instead of sorting alphabetically.
     */
    boost: Math.max(-99, 99 - index),
    detail: suggestion.detail,
    info: suggestion.info,
    type: suggestion.kind,

    /**
     * Applies the suggestion, opens requested brackets, and may continue completion.
     */
    apply(view, completion, from, to) {
      const insert = suggestion.insert ?? suggestion.label
      view.dispatch({
        changes: { from, to, insert },
        selection: { anchor: from + insert.length },
        userEvent: 'input.complete',
        annotations: pickedCompletion.of(completion),
      })
      /*
       * Brackets opened through closeBrackets are tracked, so typing the closing character steps over it.
       */
      for (const bracket of suggestion.open ?? '') {
        const spec = insertBracket(view.state, bracket)
        const head = view.state.selection.main.head
        view.dispatch(spec ?? { changes: { from: head, insert: bracket + CLOSING[bracket] }, selection: { anchor: head + 1 } })
      }
      if (suggestion.chain) startCompletion(view)
    },
  }
}

/**
 * Offer what the grammar allows at the caret; operators open by themselves after a field.
 */
function completeQuery(context: CompletionContext): CompletionResult | null {
  const { from, prefix, expectation } = completionContext(context.state.doc.toString(), context.pos)
  if (!prefix && !context.explicit && expectation.kind !== 'operator') return null
  const options = suggestionsFor(expectation).map(toCompletion)
  return options.length ? { from, options, validFor: /^(?:[A-Za-z_][A-Za-z0-9_.]*|[=!<>]*)$/ } : null
}

/**
 * Explain the field, operator, or keyword under the pointer.
 */
const documentation = hoverTooltip((view, pos, side) => {
  const doc = describeTokenAt(view.state.doc.toString(), side < 0 ? pos - 1 : pos)
  if (!doc) return null
  return {
    pos: doc.from,
    end: doc.to,
    above: true,

    /**
     * Builds the DOM content for the token's hover documentation.
     */
    create() {
      const dom = document.createElement('div')
      dom.className = 'cm-query-doc'
      const heading = dom.appendChild(document.createElement('div'))
      heading.className = 'cm-query-doc-heading'
      heading.append(Object.assign(document.createElement('code'), { textContent: doc.title }))
      heading.append(Object.assign(document.createElement('span'), { textContent: doc.detail }))
      dom.append(Object.assign(document.createElement('p'), { textContent: doc.info }))
      return { dom }
    },
  }
}, { hoverTime: 350 })

/*
 * Tooltips render outside the editor element, so their rules must not require `.cm-editor`.
 */
const TOOLTIP = '& .cm-tooltip'
/**
 * The editor theme shared by every CodeMirror editor drawn from the application's CSS tokens.
 */
export const editorTheme = EditorView.theme({
  '&': { fontSize: '13px', color: 'var(--foreground)', backgroundColor: 'transparent', maxHeight: '20rem' },
  '&.cm-focused': { outline: 'none' },
  '.cm-scroller': { fontFamily: 'var(--font-mono)', lineHeight: '1.625' },
  '.cm-content': { padding: '8px 0', minHeight: '7.5rem', caretColor: 'var(--foreground)' },
  '.cm-line': { padding: '0 12px 0 10px' },
  '.cm-gutters': {
    backgroundColor: 'transparent', color: 'color-mix(in oklab, var(--muted-foreground) 70%, transparent)',
    borderRight: '1px solid var(--border)', fontSize: '12px',
  },
  '.cm-lineNumbers .cm-gutterElement': { padding: '0 10px 0 12px', minWidth: '2.25rem' },
  '.cm-activeLine': { backgroundColor: 'color-mix(in oklab, var(--foreground) 4%, transparent)' },
  '.cm-activeLineGutter': { backgroundColor: 'color-mix(in oklab, var(--foreground) 4%, transparent)', color: 'var(--foreground)' },
  '&:not(.cm-focused) .cm-activeLine, &:not(.cm-focused) .cm-activeLineGutter': { backgroundColor: 'transparent' },
  /*
   * Rules under each line and its number; a shadow draws them without changing the measured line height.
   */
  '.cm-line, .cm-lineNumbers .cm-gutterElement': { boxShadow: 'inset 0 -1px 0 color-mix(in oklab, var(--border) 70%, transparent)' },
  '.cm-placeholder': { color: 'var(--muted-foreground)' },
  '.cm-cursor, .cm-dropCursor': { borderLeftColor: 'var(--foreground)' },
  '&.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground, .cm-selectionBackground, .cm-content ::selection': {
    backgroundColor: 'color-mix(in oklab, var(--primary) 18%, transparent)',
  },
  '&.cm-focused .cm-matchingBracket': { backgroundColor: 'color-mix(in oklab, var(--primary) 14%, transparent)', outline: 'none' },
  '&.cm-focused .cm-nonmatchingBracket': { backgroundColor: 'color-mix(in oklab, var(--destructive) 18%, transparent)' },
  [TOOLTIP]: {
    border: 'none',
    borderRadius: 'var(--radius-md)',
    backgroundColor: 'var(--popover)',
    color: 'var(--popover-foreground)',
    boxShadow: '0 0 0 1px color-mix(in oklab, var(--foreground) 10%, transparent), 0 4px 12px -2px rgb(0 0 0 / 0.12)',
    fontFamily: 'var(--font-sans)',
    fontSize: '12px',
  },
  [`${TOOLTIP}.cm-tooltip-autocomplete > ul`]: { fontFamily: 'var(--font-sans)', maxHeight: '16rem', minWidth: '16rem', padding: '4px' },
  [`${TOOLTIP}.cm-tooltip-autocomplete > ul > li`]: {
    display: 'flex', alignItems: 'center', gap: '12px', padding: '4px 8px', borderRadius: 'var(--radius-sm)', lineHeight: '1.4',
  },
  [`${TOOLTIP}.cm-tooltip-autocomplete > ul > li[aria-selected]`]: { backgroundColor: 'var(--accent)', color: 'var(--accent-foreground)' },
  '.cm-completionLabel': { fontFamily: 'var(--font-mono)' },
  '.cm-completionMatchedText': { textDecoration: 'none', fontWeight: '600' },
  '.cm-completionDetail': { marginLeft: 'auto', fontStyle: 'normal', color: 'var(--muted-foreground)' },
  '.cm-query-field .cm-completionLabel': { color: 'var(--syntax-field)' },
  '.cm-query-operator .cm-completionLabel': { color: 'var(--syntax-operator)' },
  '.cm-query-keyword .cm-completionLabel, .cm-query-value .cm-completionLabel': { color: 'var(--syntax-keyword)', fontWeight: '500' },
  [`${TOOLTIP}.cm-completionInfo`]: { padding: '6px 8px', maxWidth: '18rem', lineHeight: '1.45', color: 'var(--muted-foreground)' },
  '.cm-query-doc': { display: 'flex', flexDirection: 'column', gap: '2px', padding: '6px 8px', maxWidth: '20rem', lineHeight: '1.45' },
  '.cm-query-doc-heading': { display: 'flex', alignItems: 'baseline', gap: '8px' },
  '.cm-query-doc-heading > code': { fontFamily: 'var(--font-mono)', fontWeight: '500' },
  '.cm-query-doc-heading > span, .cm-query-doc > p': { color: 'var(--muted-foreground)', margin: '0' },
  '.cm-lintRange-error, .cm-lintRange-warning': { backgroundImage: 'none', textDecorationLine: 'underline', textDecorationStyle: 'wavy', textUnderlineOffset: '3px' },
  '.cm-lintRange-error': { textDecorationColor: 'var(--destructive)' },
  '.cm-lintRange-warning': { textDecorationColor: 'var(--warning)' },
  [`${TOOLTIP}.cm-tooltip-lint`]: { padding: '2px 0' },
  '.cm-diagnostic': { padding: '4px 8px', borderLeftWidth: '3px', marginLeft: '0' },
  '.cm-diagnostic-error': { borderLeftColor: 'var(--destructive)' },
  '.cm-diagnostic-warning': { borderLeftColor: 'var(--warning)' },
})

export interface QueryEditorOptions {
  doc: string
  placeholder: string
  disabled: boolean

  /**
   * Attributes for the editable element, such as its id and ARIA references.
   */
  attributes: Record<string, string>

  /**
   * Receives the full source whenever the editor document changes.
   */
  onChange(doc: string): void

  /**
   * Receives the 1-based line and column of the main cursor whenever it moves.
   */
  onCursor(line: number, column: number): void
}

/**
 * A mounted query editor and the operations its Vue wrapper needs.
 */
export interface QueryEditorHandle {
  view: EditorView

  /**
   * Replaces the document only when its text differs from the current source.
   */
  setDoc(doc: string): void

  /**
   * Updates editability and the read-only state together.
   */
  setDisabled(disabled: boolean): void

  /**
   * Replaces the editable element's attributes.
   */
  setAttributes(attributes: Record<string, string>): void

  /**
   * Replaces the visible lint diagnostics.
   */
  showDiagnostics(diagnostics: Diagnostic[]): void

  /**
   * Destroys the editor and removes its separately mounted tooltip layer.
   */
  destroy(): void
}

/**
 * Mount a query editor into `parent`.
 */
export function createQueryEditor(parent: HTMLElement, options: QueryEditorOptions): QueryEditorHandle {
  const editable = new Compartment()
  const attributes = new Compartment()
  /*
   * Forms and dialogs clip overflow, and field groups are CSS containers that capture fixed positioning, so
   * tooltips live in their own layer at the end of the body. Reka treats a later `data-dismissable-layer` as
   * nested in an open dialog, so clicking a suggestion neither dismisses the dialog nor is blocked by it.
   */
  const tooltipLayer = document.body.appendChild(document.createElement('div'))
  tooltipLayer.setAttribute('data-dismissable-layer', '')
  Object.assign(tooltipLayer.style, { position: 'relative', zIndex: '60', pointerEvents: 'auto' })

  /**
   * Keeps CodeMirror's editable and read-only facets in sync.
   */
  const editability = (disabled: boolean) => [EditorView.editable.of(!disabled), EditorState.readOnly.of(disabled)]

  /**
   * Reports the main cursor as a 1-based line and column to the wrapper.
   */
  const reportCursor = (state: EditorState) => {
    const head = state.selection.main.head
    const line = state.doc.lineAt(head)
    options.onCursor(line.number, head - line.from + 1)
  }
  const view = new EditorView({
    parent,
    state: EditorState.create({
      doc: options.doc,
      extensions: [
        history(),
        closeBrackets(),
        bracketMatching(),
        queryLanguage,
        syntaxHighlighting(queryHighlighting),
        lineNumbers(),
        highlightActiveLine(),
        highlightActiveLineGutter(),
        EditorView.lineWrapping,
        placeholder(options.placeholder),
        autocompletion({
          override: [completeQuery],
          icons: false,
          /*
           * Prefix matches only: field paths are long, and loose matching would mix unrelated fields in.
           */
          filterStrict: true,

          /**
           * Select the suggestion's styling class, using the value style when its type is absent.
           */
          optionClass: (completion) => `cm-query-${completion.type ?? 'value'}`,
        }),
        documentation,
        tooltips({ parent: tooltipLayer }),
        /*
         * Tab accepts an open suggestion; otherwise it keeps moving focus, as in any form field.
         */
        Prec.high(keymap.of([{ key: 'Tab', run: acceptCompletion }])),
        keymap.of([...closeBracketsKeymap, ...defaultKeymap, ...historyKeymap]),
        editable.of(editability(options.disabled)),
        attributes.of(EditorView.contentAttributes.of(options.attributes)),
        EditorView.updateListener.of((update) => {
          if (update.docChanged) options.onChange(update.state.doc.toString())
          if (update.docChanged || update.selectionSet) reportCursor(update.state)
        }),
        editorTheme,
      ],
    }),
  })
  reportCursor(view.state)
  return {
    view,

    /**
     * Replaces the document only when its text differs from the current source.
     */
    setDoc(doc) {
      if (doc !== view.state.doc.toString()) view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: doc } })
    },

    /**
     * Updates editability and the read-only state together.
     */
    setDisabled(disabled) {
      view.dispatch({ effects: editable.reconfigure(editability(disabled)) })
    },

    /**
     * Replaces the editable element's attributes.
     */
    setAttributes(next) {
      view.dispatch({ effects: attributes.reconfigure(EditorView.contentAttributes.of(next)) })
    },

    /**
     * Replaces the visible lint diagnostics.
     */
    showDiagnostics(diagnostics) {
      view.dispatch(setDiagnostics(view.state, diagnostics))
    },

    /**
     * Destroys the editor and removes its separately mounted tooltip layer.
     */
    destroy() {
      view.destroy()
      tooltipLayer.remove()
    },
  }
}
