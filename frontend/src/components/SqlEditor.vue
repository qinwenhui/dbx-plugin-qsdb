<script setup lang="ts">
// SQL 编辑器：CodeMirror 6，主题色取自 DBX tokens（宿主明暗切换即时生效），
// Ctrl/Cmd+Enter 运行。初始内容 = 草稿（store 在 init 时从 storage 恢复）。
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

const props = defineProps<{ placeholder: string }>();
const emit = defineEmits<{ (e: "run"): void }>();

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
        EditorView.lineWrapping,
        keymap.of([...defaultKeymap, ...historyKeymap, ...searchKeymap, indentWithTab]),
        EditorView.updateListener.of((u) => {
          if (u.docChanged) draftChanged(u.state.doc.toString());
        }),
        keymap.of([
          {
            key: "Mod-Enter",
            preventDefault: true,
            run: () => {
              emit("run");
              return true;
            },
          },
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
