<script setup lang="ts">
// SQL 编辑器：CodeMirror 6，主题色取自 DBX tokens（宿主明暗切换即时生效）。
// 快捷键：Ctrl/Cmd+Enter 运行（有选中只运行选中）、F5 运行、
// Ctrl/Cmd+Shift+F 格式化、Ctrl/Cmd+S 保存脚本。App 通过 defineExpose
// 的 API 取运行目标/改写文档。
import { onBeforeUnmount, onMounted, watch } from "vue";
import { EditorView, keymap, lineNumbers, highlightActiveLine, drawSelection } from "@codemirror/view";
import { EditorState, Compartment } from "@codemirror/state";
import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import { sql, PostgreSQL } from "@codemirror/lang-sql";
import { syntaxHighlighting, HighlightStyle } from "@codemirror/language";
import { tags as t } from "@lezer/highlight";
import { searchKeymap, highlightSelectionMatches } from "@codemirror/search";
import { closeBrackets, autocompletion } from "@codemirror/autocomplete";
import { state as store, draftChanged } from "../store";
import { formatSql } from "../sqlkit";

const props = defineProps<{ placeholder: string }>();
const emit = defineEmits<{
  (e: "run"): void;
  (e: "format"): void;
  (e: "save"): void;
}>();

// 编辑器主题：全部走 CSS 变量（宿主换主题无需重建编辑器）。
const editorTheme = EditorView.theme({
  "&": { backgroundColor: "var(--color-background)", color: "var(--color-foreground)" },
  ".cm-gutters": { backgroundColor: "var(--color-muted)", color: "var(--color-muted-foreground)", border: "none" },
  ".cm-activeLine": { backgroundColor: "color-mix(in srgb, var(--color-primary) 6%, transparent)" },
  ".cm-activeLineGutter": { backgroundColor: "color-mix(in srgb, var(--color-primary) 8%, transparent)" },
  ".cm-selectionBackground, &.cm-focused .cm-selectionBackground": { backgroundColor: "var(--color-primary-alpha, rgba(37,99,235,.25))" },
  ".cm-cursor": { borderLeftColor: "var(--color-foreground)" },
  ".cm-tooltip": { backgroundColor: "var(--color-popover, white)", border: "1px solid var(--color-border)", color: "var(--color-foreground)" },
  ".cm-placeholder": { color: "var(--color-muted-foreground)" },
  ".cm-panels": { backgroundColor: "var(--color-muted)", color: "var(--color-foreground)" },
});

const highlight = syntaxHighlighting(
  HighlightStyle.define([
    { tag: t.keyword, color: "var(--color-primary, #2563eb)", fontWeight: "600" },
    { tag: t.string, color: "#16a34a" },
    { tag: t.number, color: "#ea580c" },
    { tag: t.comment, color: "var(--color-muted-foreground)", fontStyle: "italic" },
    { tag: t.operator, color: "var(--color-foreground)" },
    { tag: t.punctuation, color: "var(--color-muted-foreground)" },
  ]),
);

let view: EditorView | undefined;
const langComp = new Compartment();

/** 从 Schema 树拖表进编辑器：落点插入该表的预览查询。
 * 自定义 MIME（application/x-qsdb-table）识别来源；其他拖放不拦截。 */
const TABLE_MIME = "application/x-qsdb-table";
const dragHandlers = EditorView.domEventHandlers({
  dragover(event) {
    if (event.dataTransfer && Array.from(event.dataTransfer.types).includes(TABLE_MIME)) {
      event.preventDefault();
      event.dataTransfer.dropEffect = "copy";
      return true;
    }
    return false;
  },
  drop(event, view) {
    const table = event.dataTransfer?.getData(TABLE_MIME);
    if (!table) return false;
    event.preventDefault();
    const insert = `SELECT * FROM ${table} LIMIT 100`;
    const pos = view.posAtCoords({ x: event.clientX, y: event.clientY }) ?? view.state.selection.main.head;
    view.dispatch({
      changes: { from: pos, insert },
      selection: { anchor: pos + insert.length },
    });
    return true;
  },
});

function docText(): string {
  return view ? view.state.doc.toString() : "";
}

/** 运行目标：有非空选中 → 只跑选中；否则整篇。 */
function runTarget(): { text: string; fromSelection: boolean } {
  if (!view) return { text: "", fromSelection: false };
  const sel = view.state.sliceDoc(view.state.selection.main.from, view.state.selection.main.to);
  if (sel.trim()) return { text: sel, fromSelection: true };
  return { text: docText(), fromSelection: false };
}

function setDoc(text: string) {
  if (!view) return;
  view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: text } });
}

function format() {
  if (!view) return;
  const sel = view.state.selection.main;
  const selText = view.state.sliceDoc(sel.from, sel.to);
  if (selText.trim()) {
    view.dispatch({ changes: { from: sel.from, to: sel.to, insert: formatSql(selText) } });
  } else {
    setDoc(formatSql(docText()));
  }
}

defineExpose({ runTarget, setDoc, docText, format });

onMounted(() => {
  view = new EditorView({
    parent: document.querySelector(".qsdb-editor")!,
    state: EditorState.create({
      doc: store.sql,
      extensions: [
        lineNumbers(),
        history(),
        drawSelection(),
        highlightActiveLine(),
        highlightSelectionMatches(),
        closeBrackets(),
        autocompletion(),
        langComp.of(sql({ dialect: PostgreSQL, upperCaseKeywords: true })),
        editorTheme,
        highlight,
        dragHandlers,
        EditorView.lineWrapping,
        keymap.of([...defaultKeymap, ...historyKeymap, ...searchKeymap, indentWithTab]),
        EditorView.updateListener.of((u) => {
          if (u.docChanged) draftChanged(u.state.doc.toString());
        }),
        keymap.of([
          { key: "Mod-Enter", preventDefault: true, run: () => (emit("run"), true) },
          { key: "F5", preventDefault: true, run: () => (emit("run"), true) },
          { key: "Mod-Shift-f", preventDefault: true, run: () => (emit("format"), true) },
          { key: "Mod-s", preventDefault: true, run: () => (emit("save"), true) },
        ]),
        EditorState.allowMultipleSelections.of(true),
      ],
    }),
  });
  // placeholder 扩展按需引入成本高——用空文档 + 属性提示的轻方案
});

onBeforeUnmount(() => view?.destroy());

watch(
  () => props.placeholder,
  () => undefined,
);
</script>

<template>
  <div class="qsdb-editor" :data-placeholder="props.placeholder"></div>
</template>
