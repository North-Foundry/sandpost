/**
 * CodeMirror setup for PersonaScript: a stream language that highlights statements and, inside `if (...)`, the
 * Sandpost query words with the query editor's classification; statement completion; and inline diagnostics
 * reported by the server.
 */
import { autocompletion, closeBrackets, closeBracketsKeymap, type CompletionContext, type CompletionResult } from '@codemirror/autocomplete'
import { defaultKeymap, history, historyKeymap, indentWithTab } from '@codemirror/commands'
import { HighlightStyle, StreamLanguage, bracketMatching, indentUnit, syntaxHighlighting, type StringStream } from '@codemirror/language'
import { setDiagnostics, type Diagnostic } from '@codemirror/lint'
import { Compartment, EditorState } from '@codemirror/state'
import { EditorView, highlightActiveLine, highlightActiveLineGutter, keymap, lineNumbers, placeholder, tooltips } from '@codemirror/view'
import { tags } from '@lezer/highlight'
import { characterIndexForByteOffset } from '@/api/admin'
import type { PersonaScriptDiagnostic } from '@/api/contracts'
import { asSentence } from '@/utils/errors'
import { editorTheme } from '@/features/query/editor'
import { QUERY_TOKEN_CLASSES } from '@/features/query/tokenClasses'
import { classifyQueryWord } from '@/features/query/vocabulary'

type ScriptTokenKind = 'keyword' | 'field' | 'operator' | 'string' | 'number' | 'punctuation' | 'comment'

/**
 * Tokenizer state carried across lines.
 */
interface ScriptState {
  /**
   * The quote character of an unterminated string, or null.
   */
  quote: string | null
  comment: boolean

  /**
   * Parenthesis depth inside an `if` condition; zero outside conditions.
   */
  condition: number

  /**
   * The previous word was `if`, so the next `(` opens a condition.
   */
  afterIf: boolean
}

const KEYWORDS = new Set(['open', 'wait', 'click', 'link', 'if', 'else'])
const TOKEN_TAGS = {
  keyword: tags.keyword,
  field: tags.propertyName,
  operator: tags.operator,
  string: tags.string,
  number: tags.number,
  punctuation: tags.bracket,
  comment: tags.comment,
} satisfies Record<ScriptTokenKind, unknown>
const TOKEN_CLASSES: Record<ScriptTokenKind, string> = {
  keyword: QUERY_TOKEN_CLASSES.keyword,
  field: QUERY_TOKEN_CLASSES.field,
  operator: QUERY_TOKEN_CLASSES.operator,
  string: QUERY_TOKEN_CLASSES.string,
  number: QUERY_TOKEN_CLASSES.number,
  punctuation: QUERY_TOKEN_CLASSES.punctuation,
  comment: 'text-muted-foreground italic',
}

/**
 * Read the rest of a string literal, which may continue on later lines.
 */
function readString(stream: StringStream, state: ScriptState): ScriptTokenKind {
  while (!stream.eol()) {
    const character = stream.next()
    if (character === '\\') stream.next()
    else if (character === state.quote) { state.quote = null; break }
  }
  return 'string'
}

/**
 * Classify the next token of PersonaScript source.
 */
export function scriptToken(stream: StringStream, state: ScriptState): ScriptTokenKind | null {
  if (state.comment) {
    if (stream.skipTo('*/')) { stream.match('*/'); state.comment = false } else stream.skipToEnd()
    return 'comment'
  }
  if (state.quote) return readString(stream, state)
  if (stream.eatSpace()) return null
  if (stream.match('//')) { stream.skipToEnd(); return 'comment' }
  if (stream.match('/*')) { state.comment = true; return scriptToken(stream, state) ?? 'comment' }
  const quote = stream.peek()
  if (quote === '"' || quote === "'") { stream.next(); state.quote = quote; return readString(stream, state) }
  if (state.condition > 0) {
    if (stream.eat('(')) { state.condition += 1; return 'punctuation' }
    if (stream.eat(')')) { state.condition -= 1; return 'punctuation' }
    if (stream.match(/^(?:==|!=|>=|<=|<|>)/)) return 'operator'
    if (stream.match(/^-?\d+/)) return 'number'
    const word = stream.match(/^[A-Za-z_][A-Za-z0-9_.-]*/) as RegExpMatchArray | null
    if (word) {
      const kind = classifyQueryWord(word[0])
      return kind === 'plain' ? null : kind === 'number' ? 'number' : kind
    }
    return '[]'.includes(stream.next() ?? '') ? 'punctuation' : null
  }
  if (stream.match(/^\d+(?:ms|s|m)\b/)) return 'number'
  const word = stream.match(/^[A-Za-z_][A-Za-z0-9_]*/) as RegExpMatchArray | null
  if (word) {
    state.afterIf = word[0] === 'if'
    if (KEYWORDS.has(word[0])) return 'keyword'
    return (word[0] === 'text' || word[0] === 'href') && stream.match(/^\s*:/, false) ? 'field' : null
  }
  const character = stream.next() ?? ''
  if (character === '(' && state.afterIf) { state.afterIf = false; state.condition = 1; return 'punctuation' }
  return '(){};:,'.includes(character) ? 'punctuation' : null
}

const scriptLanguage = StreamLanguage.define<ScriptState>({
  name: 'personascript',

  /**
   * Start every document outside strings, comments, and conditions.
   */
  startState: () => ({ quote: null, comment: false, condition: 0, afterIf: false }),

  /**
   * Copy the tokenizer state for incremental reparsing.
   */
  copyState: (state) => ({ ...state }),
  token: scriptToken,
  languageData: { closeBrackets: { brackets: ['(', '{', '"'] }, commentTokens: { line: '//', block: { open: '/*', close: '*/' } } },
  tokenTable: TOKEN_TAGS,
})

const scriptHighlighting = HighlightStyle.define((Object.keys(TOKEN_TAGS) as ScriptTokenKind[])
  .map((kind) => ({ tag: TOKEN_TAGS[kind], class: TOKEN_CLASSES[kind] })))

/**
 * Statement templates offered at the start of a statement.
 */
const STATEMENTS = [
  { label: 'open;', detail: 'open the message' },
  { label: 'wait 5s;', detail: 'pause' },
  { label: 'click link("a");', detail: 'CSS selector' },
  { label: 'click link(text: "");', detail: 'link text' },
  { label: 'click link(href: "**");', detail: 'URL pattern' },
  { label: 'if () {\n}', detail: 'query condition' },
] as const

/**
 * Report whether a position lies inside an `if (...)` condition, where query syntax applies instead of statements.
 * Strings and comments are skipped like the tokenizer skips them.
 */
export function insideCondition(source: string, position: number): boolean {
  let depth = 0
  let afterIf = false
  let index = 0
  while (index < position) {
    const rest = source.slice(index)
    if (rest.startsWith('//')) {
      const end = source.indexOf('\n', index)
      index = end < 0 ? position : end
      continue
    }
    if (rest.startsWith('/*')) {
      const end = source.indexOf('*/', index + 2)
      index = end < 0 ? position : end + 2
      continue
    }
    const character = source[index]!
    if (character === '"' || character === "'") {
      index += 1
      while (index < position && source[index] !== character) index += source[index] === '\\' ? 2 : 1
      index += 1
      continue
    }
    const word = /^[A-Za-z_][A-Za-z0-9_.-]*/.exec(rest)
    if (word) {
      if (depth === 0) afterIf = word[0] === 'if'
      index += word[0].length
      continue
    }
    if (character === '(') {
      if (depth > 0) depth += 1
      else if (afterIf) depth = 1
      afterIf = false
    } else if (character === ')') {
      if (depth > 0) depth -= 1
    } else if (!/\s/.test(character)) {
      afterIf = false
    }
    index += 1
  }
  return depth > 0
}

/**
 * Offer statement templates outside conditions.
 */
function completeStatement(context: CompletionContext): CompletionResult | null {
  const word = context.matchBefore(/[A-Za-z]*/)
  if (!word || (word.from === word.to && !context.explicit)) return null
  if (insideCondition(context.state.doc.toString(), context.pos)) return null
  return {
    from: word.from,
    options: STATEMENTS.map((statement, index) => ({ label: statement.label, detail: statement.detail, type: 'keyword', boost: 99 - index })),
    validFor: /^[A-Za-z]*$/,
  }
}

/**
 * Turn the server's diagnostic into an editor diagnostic over the offending word.
 */
export function scriptDiagnostics(source: string, error: PersonaScriptDiagnostic | null): Diagnostic[] {
  if (!error) return []
  const from = Math.min(characterIndexForByteOffset(source, error.position), Math.max(0, source.length - 1))
  const word = /^[^\s;(){}]+/.exec(source.slice(from))
  const to = Math.min(source.length, from + Math.max(1, word?.[0].length ?? 1))
  return [{ from, to: Math.max(to, from + (source.length ? 1 : 0)), severity: 'error', message: asSentence(error.message) }]
}

export interface ScriptEditorOptions {
  doc: string
  placeholder: string
  disabled: boolean

  /**
   * Attributes for the editable element, such as its id and ARIA references.
   */
  attributes: Record<string, string>

  /**
   * Receives the full source whenever the document changes.
   */
  onChange(doc: string): void

  /**
   * Receives the 1-based line and column of the main cursor whenever it moves.
   */
  onCursor(line: number, column: number): void
}

/**
 * A mounted script editor and the operations its Vue wrapper needs.
 */
export interface ScriptEditorHandle {
  view: EditorView

  /**
   * Replace the document only when its text differs.
   */
  setDoc(doc: string): void

  /**
   * Update editability and the read-only state together.
   */
  setDisabled(disabled: boolean): void

  /**
   * Replace the editable element's attributes.
   */
  setAttributes(attributes: Record<string, string>): void

  /**
   * Replace the visible diagnostics.
   */
  showDiagnostics(diagnostics: Diagnostic[]): void

  /**
   * Destroy the editor and its tooltip layer.
   */
  destroy(): void
}

/**
 * Mount a PersonaScript editor into `parent`.
 */
export function createScriptEditor(parent: HTMLElement, options: ScriptEditorOptions): ScriptEditorHandle {
  const editable = new Compartment()
  const attributes = new Compartment()
  /*
   * As in the query editor, tooltips live in their own body-level layer so dialogs neither clip nor dismiss them.
   */
  const tooltipLayer = document.body.appendChild(document.createElement('div'))
  tooltipLayer.setAttribute('data-dismissable-layer', '')
  Object.assign(tooltipLayer.style, { position: 'relative', zIndex: '60', pointerEvents: 'auto' })

  /**
   * Keep CodeMirror's editable and read-only facets in sync.
   */
  const editability = (disabled: boolean) => [EditorView.editable.of(!disabled), EditorState.readOnly.of(disabled)]

  /**
   * Report the main cursor as a 1-based line and column.
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
        indentUnit.of('    '),
        scriptLanguage,
        syntaxHighlighting(scriptHighlighting),
        lineNumbers(),
        highlightActiveLine(),
        highlightActiveLineGutter(),
        placeholder(options.placeholder),
        autocompletion({ override: [completeStatement], icons: false }),
        tooltips({ parent: tooltipLayer }),
        keymap.of([...closeBracketsKeymap, ...defaultKeymap, ...historyKeymap, indentWithTab]),
        editable.of(editability(options.disabled)),
        attributes.of(EditorView.contentAttributes.of(options.attributes)),
        EditorView.updateListener.of((update) => {
          if (update.docChanged) options.onChange(update.state.doc.toString())
          if (update.docChanged || update.selectionSet) reportCursor(update.state)
        }),
        editorTheme,
        EditorView.theme({ '.cm-content': { minHeight: '10rem' }, '&': { maxHeight: '22rem' } }),
      ],
    }),
  })
  reportCursor(view.state)
  return {
    view,

    /**
     * Replace the document only when its text differs.
     */
    setDoc(doc) {
      if (doc !== view.state.doc.toString()) view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: doc } })
    },

    /**
     * Update editability and the read-only state together.
     */
    setDisabled(disabled) {
      view.dispatch({ effects: editable.reconfigure(editability(disabled)) })
    },

    /**
     * Replace the editable element's attributes.
     */
    setAttributes(next) {
      view.dispatch({ effects: attributes.reconfigure(EditorView.contentAttributes.of(next)) })
    },

    /**
     * Replace the visible diagnostics.
     */
    showDiagnostics(diagnostics) {
      view.dispatch(setDiagnostics(view.state, diagnostics))
    },

    /**
     * Destroy the editor and remove its tooltip layer.
     */
    destroy() {
      view.destroy()
      tooltipLayer.remove()
    },
  }
}
