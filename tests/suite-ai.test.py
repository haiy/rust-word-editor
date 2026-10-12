"""Real model/UI checks in an owned remote Chrome tab; synthetic documents only.

WORD_TEST_URL must name the isolated Rust test server. This never opens the user's
running desktop app, never fakes a model response, and closes only its own tab.
"""
import base64
import copy
import json
import os
from pathlib import Path
import shlex
import struct
import time
import traceback
import urllib.request
import zlib

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "test-output" / "suite-ai-checks"
OUT.mkdir(parents=True, exist_ok=True)
URL = os.environ.get("WORD_TEST_URL")
if not URL:
    raise SystemExit("请设置 WORD_TEST_URL 为隔离 Rust 测试服务器，不使用生产应用")
CONFIG = dict(os.environ)
for line in (Path.home() / ".config/codex-chrome-mcp/env").read_text().splitlines():
    line = line.strip().removeprefix("export ")
    if line and not line.startswith("#") and "=" in line:
        key, value = line.split("=", 1)
        tokens = shlex.split(value)
        if tokens:
            CONFIG.setdefault(key, tokens[0])
BASE = CONFIG["CODEX_CHROME_API_URL_LAN"]
OPENER = urllib.request.build_opener(urllib.request.ProxyHandler({}))
TAB = None
CHECKS = []
RESULTS = []
ERROR = None


def call(name, **arguments):
    if TAB and name != "chrome_new_tab":
        arguments.setdefault("tab_id", TAB)
    request = urllib.request.Request(
        BASE + "/api/tools/call",
        data=json.dumps({"name": name, "arguments": arguments}).encode(),
        headers={"Authorization": "Bearer " + CONFIG["CODEX_CHROME_API_TOKEN"], "Content-Type": "application/json"},
    )
    result = json.load(OPENER.open(request, timeout=45))
    if not result.get("ok") or result.get("result", {}).get("isError"):
        raise RuntimeError("远端 Chrome 工具调用失败：" + name)
    raw = "".join(block.get("text", "") for block in result["result"]["content"] if block["type"] == "text")
    return json.loads(raw)


def evaluate(expression):
    result = call("chrome_cdp", method="Runtime.evaluate", params={"expression": expression, "returnByValue": True, "awaitPromise": True})
    if result.get("exceptionDetails"):
        raise AssertionError(result["exceptionDetails"])
    return result.get("result", {}).get("value")


def click(selector):
    call("chrome_click", selector=selector)


def wait_for(expression, seconds=20):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        value = evaluate(expression)
        if value:
            return value
        time.sleep(.5)
    raise AssertionError("等待状态超时：" + expression)


def check(condition, description):
    if not condition:
        raise AssertionError(description)
    CHECKS.append(description)
    print("PASS " + description, flush=True)


def screenshot(name):
    result = call("chrome_cdp", method="Page.captureScreenshot", params={"format": "png"})
    (OUT / name).write_bytes(base64.b64decode(result["data"]))


def model():
    return evaluate("window.rustWord.document()")


def load(document, name):
    evaluate("window.rustWord.loadDocument(" + json.dumps({"document": document, "name": name}, ensure_ascii=False) + ")")
    return model()


def text_nodes(document):
    result = []

    def visit(node, path):
        if node["type"] == "text":
            result.append((path, node["text"]))
        for i, child in enumerate(node.get("content", [])):
            visit(child, path + (i,))

    visit(document, ())
    return result


def structure_without_text(document):
    result = copy.deepcopy(document)

    def visit(node):
        if node["type"] == "text":
            node["text"] = "__TEXT__"
        for child in node.get("content", []):
            visit(child)

    visit(result)
    return result


def image_nodes(document):
    found = []

    def visit(node):
        if node["type"] == "image":
            found.append(node)
        for child in node.get("content", []):
            visit(child)

    visit(document)
    return found


def png_data():
    def chunk(kind, payload):
        return struct.pack(">I", len(payload)) + kind + payload + struct.pack(">I", zlib.crc32(kind + payload) & 0xffffffff)
    raw = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 2, 2, 8, 6, 0, 0, 0))
    raw += chunk(b"IDAT", zlib.compress((b"\x00" + b"\x30\x80\xc0\xff" * 2) * 2)) + chunk(b"IEND", b"")
    return "data:image/png;base64," + base64.b64encode(raw).decode()


def paragraph(value, marks=None):
    item = {"type": "text", "text": value}
    if marks:
        item["marks"] = marks
    return {"type": "paragraph", "content": [item]}


def complex_document():
    return {"type": "doc", "content": [
        {"type": "heading", "attrs": {"level": 2, "textAlign": "center"}, "content": [{"type": "text", "text": "季度项目会议"}]},
        {"type": "paragraph", "attrs": {"lineHeight": "1.8", "spaceBefore": 6, "spaceAfter": 12}, "content": [
            {"type": "text", "text": "请准备季度报告", "marks": [{"type": "bold"}]},
            {"type": "text", "text": "，会议将在周一举行。", "marks": [{"type": "textStyle", "attrs": {"fontSize": "14pt", "color": "#205040"}}]},
        ]},
        {"type": "table", "content": [
            {"type": "tableRow", "content": [
                {"type": "tableHeader", "content": [paragraph("工作项目")]},
                {"type": "tableHeader", "content": [paragraph("完成时间")]},
            ]},
            {"type": "tableRow", "content": [
                {"type": "tableCell", "content": [paragraph("风险清单", [{"type": "bold"}])]},
                {"type": "tableCell", "content": [paragraph("周五")]},
            ]},
        ]},
        {"type": "image", "attrs": {"src": png_data(), "alt": "合成验收图片", "width": 64, "height": 32}},
        paragraph("本文件仅用于自动化功能验收，不含用户文档。"),
    ]}


def begin(action, instruction=None):
    click('[data-tab="tools"]')
    click('[data-action="' + action + '"]')
    wait_for("!document.querySelector('#suite-overlay').classList.contains('hidden')")
    if instruction is not None:
        call("chrome_type", selector="#format-instruction", text=instruction, clear=True)
    click("#document-ai-generate")
    wait_for("document.querySelector('#suite-overlay').classList.contains('busy')")
    check(evaluate("document.querySelector('#document-ai-apply').disabled"), "生成期间禁止应用未完成结果：" + action)


def finish(action):
    wait_for("!document.querySelector('#suite-overlay').classList.contains('busy')", seconds=240)
    state = evaluate("({status:document.querySelector('#suite-status').textContent,error:document.querySelector('#suite-status').classList.contains('error'),preview:document.querySelector('#document-ai-preview').textContent,disabled:document.querySelector('#document-ai-apply').disabled})")
    check(not state["error"] and bool(state["preview"].strip()), "真实模型返回整文预览：" + action)
    response = evaluate("window.__suiteAiTrace.at(-1)")
    check(bool(response) and response["command"] in ["aiTranslate", "aiFormat"] and response["data"]["provider"] == "codex", "预览来自真实 Codex Rust 返回值：" + action)
    RESULTS.append({"action": action, "state": state, "response": response})
    return state


try:
    call("chrome_status")
    TAB = call("chrome_new_tab", url=URL)["tab"]["id"]
    (OUT / "owned-tab-id.txt").write_text(TAB)
    wait_for("!!window.rustWord?.suite")
    if evaluate("!document.querySelector('#modal').classList.contains('hidden')"):
        check(evaluate("document.querySelector('#modal-title').textContent.includes('恢复')"), "启动弹窗仅为隔离测试草稿恢复")
        click("#modal-actions button:first-child")
    settings = evaluate("window.rustWord.invoke('aiSettings')")
    check(settings["provider"] == "codex", "隔离服务器默认使用已登录本机 Codex")
    # Observe real fetch replies without replacing any response or altering payloads.
    evaluate("window.__suiteAiTrace=[];window.__suiteOriginalFetch=window.fetch;window.fetch=async function(...args){const response=await window.__suiteOriginalFetch.apply(this,args);try{const sent=JSON.parse(args[1]?.body||'null');if(['aiTranslate','aiFormat'].includes(sent?.command)){const received=await response.clone().json();window.__suiteAiTrace.push({command:sent.command,data:received.data,error:received.error});}}catch(e){}return response;}")

    source = load(complex_document(), "全文翻译合成验收")
    expected_nodes = text_nodes(source)
    check(len(expected_nodes) >= 8 and len(image_nodes(source)) == 1, "合成文档包含标题、文字格式、表格及图片")
    begin("translateDocument")
    check(model() == source, "点击生成后正文保持原样")
    state = finish("translateDocument")
    check(model() == source and not state["disabled"], "生成完整译文只更新预览，等待明确应用")
    check("Monday" in state["preview"], "译文预览包含真实英文会议日期")
    check(evaluate("document.querySelectorAll('#document-ai-preview table').length===1 && document.querySelectorAll('#document-ai-preview img').length===1 && document.querySelectorAll('#document-ai-preview strong').length>=2"), "翻译预览实际渲染表格、图片和粗体格式")
    screenshot("translation-preview.png")
    click("#document-ai-apply")
    applied = model()
    check(structure_without_text(applied) == structure_without_text(source), "翻译应用保留全部文档结构、文字 marks、表格、图片与段落格式")
    check([p for p, _ in text_nodes(applied)] == [p for p, _ in expected_nodes], "译文按原文字节点路径一一对应")
    check(text_nodes(applied) != expected_nodes, "明确应用才把原文替换为译文")
    click("#suite-close")
    click('[data-action="undo"]')
    check(model() == source, "一次撤销完整恢复翻译前模型")

    layout_source = complex_document()
    layout_source["content"][0]["type"] = "paragraph"
    layout_source["content"][0]["attrs"] = {"textAlign": "left", "spaceBefore": 0, "spaceAfter": 0}
    layout_source["content"][0]["content"][0]["text"] = "季度项目计划"
    layout_source["content"].insert(1, paragraph("一、项目会议安排"))
    source = load(layout_source, "AI排版合成验收")
    before_text = text_nodes(source)
    begin("formatDocument", "中文项目报告：总标题20磅黑体居中，章节标题分层，正文12磅宋体、1.5倍行距。保留正文内容与表格图片。")
    state = finish("formatDocument")
    check(model() == source and not state["disabled"], "真实 AI 排版仅生成建议预览，未提前修改正文")
    check(evaluate("[...document.querySelectorAll('#document-ai-preview [style]')].some(el=>el.style.fontSize || el.style.lineHeight || el.style.marginTop || el.style.marginBottom)"), "排版预览真实显示字号、行距或段落间距")
    screenshot("formatting-preview.png")
    click("#document-ai-apply")
    applied = model()
    check(text_nodes(applied) == before_text, "AI 排版应用不改写任何文字节点内容与顺序")
    check(image_nodes(applied) == image_nodes(source), "AI 排版保留原图片数据与尺寸")
    check(applied["content"][0]["type"] == "heading" and applied != source, "AI 排版实际更新标题层级与格式属性")
    check(evaluate("document.querySelector('.ProseMirror').querySelector('h1')!==null"), "应用后的真实编辑器出现可编辑标题")
    click("#suite-close")
    click('[data-action="undo"]')
    check(model() == source, "一次撤销完整恢复 AI 排版前模型")

    source = load({"type": "doc", "content": [paragraph("团队将在周一讨论项目进度，请提前准备风险清单。")]}, "生成期间修改合成验收")
    begin("translateDocument")
    evaluate("window.rustWord.editor.commands.insertContentAt(window.rustWord.editor.state.doc.content.size-1,' 手动编辑保留。')")
    changed = model()
    check(changed != source, "真实生成期间编辑器接收一次手动编辑")
    state = finish("translateDocument-stale")
    check(state["disabled"] and "正文已经变化" in state["status"], "生成期间正文变化后禁用过期建议应用")
    check(model() == changed, "过期翻译结果没有覆盖手动修改")
    screenshot("stale-result-blocked.png")
    click("#suite-close")
except Exception:
    ERROR = traceback.format_exc()
    print(ERROR, flush=True)
    if TAB:
        try:
            screenshot("failure.png")
        except Exception:
            pass
finally:
    report = {"ok": ERROR is None, "syntheticDocumentsOnly": True, "realModel": True, "serverUrl": URL, "checks": CHECKS, "aiResults": RESULTS, "error": ERROR}
    (OUT / "report.json").write_text(json.dumps(report, ensure_ascii=False, indent=2))
    if TAB:
        try:
            call("chrome_close_tab")
        except Exception:
            pass
if ERROR:
    raise SystemExit(1)
