"use strict";
const invoke = window.__TAURI__.core.invoke;
const $ = (id) => document.getElementById(id);
let busy = false;
let objectMode = false;

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
  objectMode = await invoke("todo_object_mode");
  const access = objectMode ? (await syncRequest({ action: "object_access" })).data : [];
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
    const permissions = access.find((item) => item.id === todo.id);
    if (permissions && (!permissions.rights.change || permissions.status !== "active")) { check.disabled = true; check.dataset.locked = "true"; }
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
    if (permissions && (!permissions.rights.change || permissions.status !== "active")) { edit.disabled = true; edit.dataset.locked = "true"; }
    const remove = button("Удалить", async () => {
      await invoke("todo_delete", { id: todo.id });
      status("Задача удалена на устройстве. Изменение ожидает синхронизации.");
    }, "delete");
    if (permissions && (!permissions.rights.delete || permissions.status !== "active")) { remove.disabled = true; remove.dataset.locked = "true"; }
    row.append(check, title, edit, remove);
    $("todos").append(row);
  }
  const retained = access.filter((item) => item.status !== "active" || !item.rights.change);
  $("object-access").hidden = retained.length === 0;
  $("object-access-list").replaceChildren();
  for (const item of retained) {
    const card = document.createElement("div");
    card.className = "conflict";
    const label = document.createElement("p");
    const names = { revoked: "Доступ отозван", deleted: "Объект удалён", review: "Правки ожидают вашего решения", discarded: "Локальные данные удалены", active: "Только чтение" };
    label.textContent = `${names[item.status] || item.status}: ${item.id}${item.submitted ? " · исход отправки пока неизвестен" : ""}`;
    card.append(label);
    if (item.has_draft) {
      card.append(button("Показать сохранённые правки", async () => {
        const result = await syncRequest({ action: "retained_draft", model: item.model, id: item.id });
        const preview = document.createElement("pre");
        preview.textContent = JSON.stringify(result.data, null, 2);
        card.append(preview);
        return false;
      }));
    }
    if (item.status === "review" && item.rights.read && item.rights.change) {
      card.append(button("Разрешить отправку моих правок", async () => {
        await syncRequest({ action: "resume_object", model: item.model, id: item.id });
        status("Правки подтверждены. Нажмите «Синхронизировать» для отправки.");
      }));
    }
    if (!item.rights.read && !item.submitted && item.status !== "discarded") {
      card.append(button("Удалить сохранённую копию", async () => {
        if (!window.confirm("Удалить локальную копию и её неотправленные правки?")) return false;
        await syncRequest({ action: "discard_retained_draft", model: item.model, id: item.id });
        status("Локальная копия удалена.");
      }, "delete"));
    }
    $("object-access-list").append(card);
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
    document.querySelectorAll("button, input").forEach((node) => { node.disabled = node.dataset.locked === "true"; });
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
      status("Есть конфликт, отзыв доступа или правки, требующие решения. Посмотрите панели ниже.", true);
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
