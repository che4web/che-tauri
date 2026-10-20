"use strict";
const invoke = window.__TAURI__.core.invoke;
const $ = (id) => document.getElementById(id);
let busy = false;

function status(message, error = false) {
  $("status").textContent = message;
  $("status").classList.toggle("error", error);
}

function button(text, action, className = "") {
  const node = document.createElement("button");
  node.type = "button";
  node.textContent = text;
  node.className = className;
  node.onclick = () => run(action);
  return node;
}

async function syncRequest(request) {
  return invoke("che_model_sync", { request });
}

async function refresh() {
  const todos = await invoke("todo_list");
  $("count").textContent = `${todos.filter((todo) => !todo.done).length} осталось / ${todos.length} всего`;
  $("empty").hidden = todos.length > 0;
  $("todos").replaceChildren();
  for (const todo of todos) {
    const row = document.createElement("li");
    row.classList.toggle("done", todo.done);
    const check = document.createElement("input");
    check.type = "checkbox";
    check.checked = todo.done;
    check.setAttribute("aria-label", `Выполнено: ${todo.title}`);
    check.onchange = () => run(async () => {
      await invoke("todo_update", { id: todo.id, done: check.checked });
      status("Сохранено на устройстве. Нажмите «Синхронизировать», чтобы отправить изменения.");
    });
    const title = document.createElement("span");
    title.className = "task-title";
    title.textContent = todo.title;
    const edit = button("Изменить", async () => {
      const form = document.createElement("form");
      form.className = "edit-form";
      const input = document.createElement("input");
      input.value = todo.title;
      input.maxLength = 200;
      input.required = true;
      input.setAttribute("aria-label", "Новое название");
      const save = document.createElement("button");
      save.type = "submit";
      save.textContent = "Сохранить";
      form.onsubmit = (event) => {
        event.preventDefault();
        run(async () => {
          await invoke("todo_update", { id: todo.id, title: input.value });
          status("Название сохранено на устройстве.");
        });
      };
      form.append(input, save);
      row.replaceChildren(form);
      input.focus();
      // Keep the editor until submit; this action needs no database refresh.
      return false;
    });
    row.append(check, title, edit, button("Удалить", async () => {
      await invoke("todo_delete", { id: todo.id });
      status("Задача удалена на устройстве. Изменение ожидает синхронизации.");
    }, "delete"));
    $("todos").append(row);
  }
  const response = await syncRequest({ action: "conflicts" });
  $("conflicts").hidden = response.data.length === 0;
  $("conflict-list").replaceChildren();
  for (const conflict of response.data) {
    const card = document.createElement("div");
    card.className = "conflict";
    for (const [label, record] of [["На устройстве", conflict.local], ["На сервере", conflict.remote.record]]) {
      const p = document.createElement("p");
      p.textContent = `${label}: ${record ? JSON.stringify(record, null, 2) : "Удалено"}`;
      card.append(p);
    }
    for (const [label, strategy] of [["Оставить мою", "local"], ["Принять серверную", "remote"]]) {
      card.append(button(label, async () => {
        await syncRequest({ action: "resolve", model: conflict.model, id: conflict.id, resolution: { strategy } });
        status("Конфликт разрешён. Синхронизируйте снова для отправки выбранной версии.");
      }));
    }
    $("conflict-list").append(card);
  }
}

async function run(action) {
  if (busy) return;
  busy = true;
  document.querySelectorAll("button, input").forEach((node) => { node.disabled = true; });
  try {
    if (await action() !== false) await refresh();
  } catch (error) {
    status(error.detail || error.message || String(error), true);
    // A failed HTTP step never discards local edits. Refresh local state only.
    try { await refresh(); } catch (_) { /* Keep the original error visible. */ }
  } finally {
    busy = false;
    document.querySelectorAll("button, input").forEach((node) => { node.disabled = false; });
  }
}

$("create").onsubmit = (event) => {
  event.preventDefault();
  run(async () => {
    await invoke("todo_create", { title: $("title").value });
    $("title").value = "";
    status("Задача сохранена на устройстве. Можно продолжать без сети.");
  });
};

$("sync").onclick = () => run(async () => {
  status("Синхронизация…");
  let sent = 0;
  let received = 0;
  // Bounded so a busy journal or unresolved dependency cannot spin forever.
  for (let step = 0; step < 30; step++) {
    const { data: report } = await syncRequest({ action: "sync_once" });
    sent += report.accepted_operations;
    received += report.received_changes;
    if (report.blocked || report.conflicts || report.rejected_operations) {
      status("Есть конфликт или отклонённое изменение. Выберите версию ниже.", true);
      return;
    }
    if (!report.has_more && report.accepted_operations === 0 && report.received_changes === 0) {
      status(`Синхронизировано. Отправлено: ${sent}, получено: ${received}.`);
      return;
    }
  }
  status("Часть изменений синхронизирована. Нажмите кнопку ещё раз.");
});

run(async () => { status("Локальная база готова. Изменения сохраняются даже без сети."); });
