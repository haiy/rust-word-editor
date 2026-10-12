// Original, offline templates. No WPS template content or brand assets are used.
const text = (value, marks = []) => ({ type: 'text', text: value, ...(marks.length ? { marks } : {}) });
const p = (value = '', attrs = {}) => ({ type: 'paragraph', attrs: { spaceBefore: 0, spaceAfter: 8, ...attrs }, content: value ? [text(value)] : [] });
const h = (level, value) => ({ type: 'heading', attrs: { level, spaceAfter: level === 1 ? 16 : 10, keepNext: true }, content: [text(value)] });
const list = items => ({ type: 'bulletList', content: items.map(value => ({ type: 'listItem', content: [p(value, { spaceAfter: 4 })] })) });
const table = rows => ({ type: 'table', content: rows.map((row, i) => ({ type: 'tableRow', content: row.map(value => ({ type: i ? 'tableCell' : 'tableHeader', attrs: { colspan: 1, rowspan: 1 }, content: [p(value, { spaceAfter: 0, lineHeight: '1.5' })] })) })) });
const doc = content => ({ type: 'doc', attrs: { orientation: 'portrait', marginTop: 20, marginBottom: 20, marginLeft: 20, marginRight: 20 }, content });
const date = new Intl.DateTimeFormat(window.__rustWordBoot?.locale || 'en', { year: 'numeric', month: 'long', day: 'numeric' }).format(new Date());

export const builtinTemplates = [
  {
    id: 'builtin-meeting', title: '会议纪要', category: '工作', description: '把讨论、决策和待办整理成一份可执行的记录。', accent: '#376ba8',
    document: doc([
      h(1, '会议纪要'), p(`会议主题：[填写主题]     日期：${date}`), p('参会人员：[姓名 / 部门]     记录人：[姓名]'),
      h(2, '一、会议目标'), p('明确本次会议需要解决的问题，以及达成共识后的下一步。'),
      h(2, '二、讨论要点'), list(['议题一：[背景、主要意见与讨论结果]', '议题二：[数据、风险与待确认事项]']),
      h(2, '三、决策与共识'), p('[记录已经达成的决定，区分未决事项，不把建议写成承诺。]'),
      h(2, '四、行动清单'), table([['行动事项', '负责人', '截止日期'], ['[具体交付物]', '[姓名]', '[日期]'], ['[下一步工作]', '[姓名]', '[日期]']]),
      h(2, '五、下次跟进'), p('[下次会议时间、检查方式和需要补充的材料。]'), p(),
    ]),
  },
  {
    id: 'builtin-weekly', title: '工作周报', category: '工作', description: '用成果、进度和计划，让团队迅速看懂这一周。', accent: '#408978',
    document: doc([
      h(1, '工作周报'), p('汇报人：[姓名]     团队：[部门]     周期：[起止日期]'),
      h(2, '本周成果'), list(['[完成事项]：说明结果、影响和可验证的交付物。', '[关键进展]：说明相较上周的变化。']),
      h(2, '项目进度'), table([['项目', '当前进度', '下一步'], ['[项目 A]', '[已完成 / 进行中]', '[具体行动]'], ['[项目 B]', '[进度与风险]', '[预计完成时间]']]),
      h(2, '问题与支持需求'), p('[说明阻碍、已尝试的办法，以及需要谁提供什么支持。]'),
      h(2, '下周计划'), list(['优先级 1：[目标、负责人和完成时间]', '优先级 2：[目标、负责人和完成时间]']), p(),
    ]),
  },
  {
    id: 'builtin-plan', title: '项目计划', category: '计划', description: '从目标到里程碑，安排范围、资源和验收标准。', accent: '#7564a3',
    document: doc([
      h(1, '项目计划书'), p('项目名称：[填写名称]     负责人：[姓名]     版本：1.0'),
      h(2, '项目背景与目标'), p('[描述现状、需要解决的问题、受益者，以及可衡量的目标。]'),
      h(2, '工作范围'), list(['本期交付：[功能、材料或服务]', '验收标准：[可观察的完成条件]', '范围边界：[本期暂不包含的内容]']),
      h(2, '里程碑'), table([['阶段', '交付物', '时间', '负责人'], ['准备', '[调研与方案]', '[日期]', '[姓名]'], ['实施', '[核心交付]', '[日期]', '[姓名]'], ['验收', '[验收记录]', '[日期]', '[姓名]']]),
      h(2, '资源与风险'), p('[人员、预算、依赖和主要风险；为每项风险指定预案。]'),
      h(2, '沟通与验收'), p('[同步频率、决策人、变更处理和最终验收方式。]'), p(),
    ]),
  },
  {
    id: 'builtin-report', title: '业务分析报告', category: '报告', description: '先给结论，再用事实、比较和行动建议支撑。', accent: '#b47d43',
    document: doc([
      h(1, '业务分析报告'), p('主题：[分析对象]     编写人：[姓名]     日期：[日期]'),
      h(2, '核心结论'), p('[用一段话说明主要发现、依据，以及对业务的影响。]'),
      h(2, '分析背景与口径'), p('[说明分析范围、时间区间、数据来源和指标定义。]'),
      h(2, '关键指标'), table([['指标', '本期', '对比期', '变化'], ['[指标一]', '[数值]', '[数值]', '[幅度]'], ['[指标二]', '[数值]', '[数值]', '[幅度]']]),
      h(2, '发现与原因'), list(['发现一：[事实与证据]；可能原因：[解释及待验证项]。', '发现二：[事实与证据]；业务影响：[具体结果]。']),
      h(2, '行动建议'), table([['建议', '预期结果', '负责人', '完成时间'], ['[具体行动]', '[衡量方式]', '[姓名]', '[日期]']]),
      h(2, '限制与补充材料'), p('[数据缺口、尚未确认的问题和附录。]'), p(),
    ]),
  },
  {
    id: 'builtin-resume', title: '个人简历', category: '个人', description: '清晰呈现经历、项目成果和岗位相关能力。', accent: '#577f91',
    document: doc([
      h(1, '[你的姓名]'), p('求职方向：[岗位名称]'), p('电话：[联系方式]     邮箱：[邮箱]     城市：[城市]'),
      h(2, '个人简介'), p('[用 2–3 句说明专业背景、相关经验和最能体现价值的成果。]'),
      h(2, '工作经历'), h(3, '[公司名称] · [职位]'), p('[开始年月] — [结束年月 / 至今]'),
      list(['[职责与成果]：使用动作、规模和可核验的结果描述。', '[改进与影响]：说明你解决了什么问题，以及产生了什么变化。']),
      h(2, '项目经历'), h(3, '[项目名称]'), p('角色：[承担职责]     时间：[起止日期]'), p('[项目目标、你的行动、成果与使用的技术或方法。]'),
      h(2, '教育背景'), p('[学校] · [专业] · [学历]     [起止日期]'),
      h(2, '技能与证书'), list(['[岗位相关技能及熟练程度]', '[证书、语言或作品链接]']), p(),
    ]),
  },
  {
    id: 'builtin-letter', title: '商务函件', category: '工作', description: '正式但易读的往来函件，明确诉求与回复时间。', accent: '#4e718f',
    document: doc([
      h(1, '关于[事项]的函'), p('致：[公司 / 部门 / 收件人]'), p('您好：'),
      p('[简要说明来函背景和双方已有的沟通或合作。]', { firstLineIndent: 24 }),
      p('[准确描述本次事项、需要确认的内容，以及具体请求。]', { firstLineIndent: 24 }),
      p('[说明建议安排、所需材料和希望收到回复的时间。]', { firstLineIndent: 24 }),
      p('感谢您的支持与配合，期待您的回复。', { firstLineIndent: 24 }), p(),
      p('[发件单位 / 姓名]', { textAlign: 'right' }), p('[日期]', { textAlign: 'right' }), p('联系方式：[电话 / 邮箱]'), p(),
    ]),
  },
  {
    id: 'builtin-reading', title: '读书笔记', category: '学习', description: '提炼观点、记录证据，再转成自己的行动。', accent: '#9c7867',
    document: doc([
      h(1, '读书笔记'), p('书名：[书名]     作者：[作者]     阅读日期：[日期]'),
      h(2, '这本书解决什么问题'), p('[用自己的话总结核心问题、目标读者和适用场景。]'),
      h(2, '三个关键观点'), list(['观点一：[主张、依据和页码]', '观点二：[主张、依据和页码]', '观点三：[主张、依据和页码]']),
      h(2, '我的理解与疑问'), p('[联系已有经验，写出你赞同、不确定或希望进一步验证的内容。]'),
      h(2, '可尝试的行动'), table([['行动', '应用场景', '验证方式'], ['[小规模尝试]', '[何时何地]', '[什么结果说明有效]']]),
      h(2, '摘录与出处'), p('[记录短摘录与页码，区分作者原话和你的转述。]'), p(),
    ]),
  },
  {
    id: 'builtin-study', title: '学习计划', category: '计划', description: '把学习目标拆成每周练习和可检验的成果。', accent: '#6b8c56',
    document: doc([
      h(1, '学习计划'), p('学习主题：[主题]     周期：[起止日期]'),
      h(2, '目标与基础'), p('[说明现有基础，以及完成后希望独立完成的具体任务。]'),
      h(2, '每周安排'), table([['时间', '学习内容', '实践任务', '完成证据'], ['第 1 周', '[基础]', '[练习]', '[作品 / 测验]'], ['第 2 周', '[进阶]', '[练习]', '[作品 / 测验]'], ['第 3 周', '[综合]', '[真实任务]', '[展示与反馈]']]),
      h(2, '资源与方法'), list(['主要资源：[课程、书籍或文档]', '练习安排：[固定时间与每次时长]', '反馈方式：[检查、复盘或同伴评价]']),
      h(2, '复盘记录'), p('[记录每周完成情况、遇到的问题，以及下一周的调整。]'), p(),
    ]),
  },
];

export function previewNodes(nodes, parent) {
  for (const node of nodes || []) {
    if (node.type === 'text') { parent.append(document.createTextNode(node.text || '')); continue; }
    if (node.type === 'image') { const caption = document.createElement('p'); caption.textContent = '[图片]'; parent.append(caption); continue; }
    const tag = { paragraph: 'p', heading: `h${Math.min(3, node.attrs?.level || 2)}`, bulletList: 'ul', orderedList: 'ol', listItem: 'li', table: 'table', tableRow: 'tr', tableCell: 'td', tableHeader: 'th', blockquote: 'blockquote', horizontalRule: 'hr', pageBreak: 'hr' }[node.type] || 'div';
    const el = document.createElement(tag);
    if (node.attrs?.textAlign) el.style.textAlign = node.attrs.textAlign;
    previewNodes(node.content, el); parent.append(el);
  }
}
