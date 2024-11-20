use crate::chatapi::grop::ApiGroq;
use crate::error::ChapResult;
use crate::fuzzy::Match;
use crate::text::LineMeta;
use crate::text::SimpleString;
use crate::text::SimpleTextEngine;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use crossterm::execute;
use crossterm::terminal::disable_raw_mode;
use crossterm::terminal::enable_raw_mode;
use crossterm::terminal::LeaveAlternateScreen;
use crossterm::{
    cursor::{self, MoveTo},
    event::{self, Event, KeyCode},
    style::{self, Print},
    terminal::{self, Clear, ClearType},
    ExecutableCommand,
};
use fastembed::TextEmbedding;
use galois::Tensor;
use groq_api_rs::completion::client::Groq;
use memmap2::Mmap;
use once_cell::sync::Lazy;
use ratatui::prelude::Constraint;
use ratatui::prelude::CrosstermBackend;
use ratatui::prelude::Direction;
use ratatui::prelude::Layout;
use ratatui::prelude::Position;
use ratatui::prelude::Rect;
use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::symbols::line;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::text::Text;
use ratatui::widgets::Block;
use ratatui::widgets::Borders;
use ratatui::widgets::List;
use ratatui::widgets::ListItem;
use ratatui::widgets::ListState;
use ratatui::widgets::Paragraph;
use ratatui::Terminal;
use std::default;
use std::io;
use std::io::Write;
use std::mem;
use std::process::exit;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::time::Duration;
use unicode_width::UnicodeWidthStr;
use vectorbase::collection::Collection;

pub(crate) struct ChapUI {
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
    navi: Navigation,
    tv: TextView,
    fuzzy_inp: FuzzyInput,
    chat_tv: TextView,
    chat_inp: ChatInput,
    focus: Focus,
    prompt_tx: mpsc::Sender<String>,
    vb: Collection,
    embed_model: Arc<TextEmbedding>,
}

enum FocusType {
    TxtFuzzy,
    Chat,
}

//焦点
struct Focus {
    current_focus: usize,
}

impl Focus {
    // 创建一个新的 Focus 实例
    fn new() -> Self {
        Focus { current_focus: 0 }
    }

    // 切换焦点
    fn next(&mut self) {
        self.current_focus = (self.current_focus + 1) % mem::variant_count::<FocusType>();
        // 0到3循环
    }

    fn get_colors(&self) -> (Color, Color, Color, Color) {
        let base = Color::White;
        let highlight = Color::Yellow;
        match self.current() {
            FocusType::TxtFuzzy => (highlight, highlight, base, base),
            FocusType::Chat => (base, base, highlight, highlight),
        }
    }

    // 获取当前焦点
    fn current(&self) -> FocusType {
        match self.current_focus {
            0 => FocusType::TxtFuzzy,
            1 => FocusType::Chat,
            _ => {
                todo!()
            }
        }
    }
}

struct Prompt {
    prompt: String,
    _id: String,
}

impl Prompt {
    fn prompt(&self) -> &str {
        &self.prompt
    }
    fn _id(&self) -> &str {
        &self._id
    }
}

struct ChatItemIndex(usize, usize);

impl ChatItemIndex {
    fn start(&self) -> usize {
        self.0
    }
    fn end(&self) -> usize {
        self.1
    }
}

#[derive(Default)]
// 聊天框的类型
enum ChatType {
    #[default]
    ChatTv,
    Promt,
    Pattern,
}

impl ChapUI {
    pub(crate) fn new(
        prompt_tx: mpsc::Sender<String>,
        vb: Collection,
        embed_model: Arc<TextEmbedding>,
    ) -> ChapResult<ChapUI> {
        let mut stdout = std::io::stdout();
        execute!(stdout, crossterm::terminal::EnterAlternateScreen)?;
        enable_raw_mode()?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;
        // 显示光标
        io::stdout().execute(cursor::Show)?;
        // 清空终端屏幕
        terminal.clear()?;
        // 获取终端尺寸
        let size = terminal.size()?;

        // 终端高度
        let terminal_height = size.height as usize;
        // 终端宽度
        let terminal_width = size.width as usize;

        // 文本框显示内容的高度
        let tv_heigth = terminal_height - 2;
        // 文本框显示内容的宽度
        let tv_width = (terminal_width as f32 * 0.6) as usize - 3;

        let chat_tv_width = (terminal_width as f32 * 0.4) as usize - 3;

        let max_line = terminal_height - 3;

        let rect = Rect::new(0, 0, size.width, size.height);
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(60), Constraint::Percentage(40)].as_ref())
            .split(rect);

        //文本框和输入框
        let left_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(3), Constraint::Length(2)].as_ref())
            .split(chunks[0]); // chunks[1] 是左侧区域

        //LLM聊天和输入框
        let right_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(3), Constraint::Length(2)].as_ref())
            .split(chunks[1]); // chunks[1] 是左侧区域

        //导航栏和文本框
        let nav_text_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(1), Constraint::Percentage(100)].as_ref())
            .split(left_chunks[0]); // chunks[1] 是左侧区域

        let search_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(1), Constraint::Percentage(100)].as_ref())
            .split(left_chunks[1]); // chunks[1] 是左侧区域

        let navi = Navigation {
            max_line: max_line,
            min_line: 0,
            cur_line: 0,
            rect: nav_text_chunks[0],
            select_line: None,
        };

        let tv = TextView {
            height: tv_heigth,
            width: tv_width,
            scroll: 1,
            rect: nav_text_chunks[1],
        };

        let fuzzy_inp = FuzzyInput {
            input: String::new(),
            rect: search_chunks[1],
        };

        let chat_tv = TextView {
            height: tv_heigth,
            width: chat_tv_width,
            scroll: 1,
            rect: right_chunks[0],
        };

        let chat_inp = ChatInput {
            input: String::new(),
            rect: right_chunks[1],
        };

        Ok(ChapUI {
            terminal: terminal,
            navi: navi,
            tv: tv,
            fuzzy_inp: fuzzy_inp,
            chat_tv: chat_tv,
            chat_inp: chat_inp,
            focus: Focus::new(),
            prompt_tx: prompt_tx,
            vb: vb,
            embed_model: embed_model,
        })
    }

    pub(crate) async fn render(
        &mut self,
        bytes: Mmap,
        mut llm_res_rx: mpsc::Receiver<String>,
    ) -> ChapResult<()> {
        let mut eg = SimpleTextEngine::new(bytes, self.tv.get_height(), self.tv.get_width());

        let mut chat_eg = SimpleTextEngine::new(
            SimpleString::new(String::with_capacity(1024)),
            self.chat_tv.get_height(),
            self.chat_tv.get_width(),
        );
        let mut chat_index: usize = 0;
        let mut chat_scorll: usize = 0;
        let mut chat_item: Vec<ChatItemIndex> = Vec::new();
        let mut chat_type = ChatType::default();
        loop {
            let line_meta = {
                let (inp, is_exact) = self.fuzzy_inp.get_inp_exact();
                let (tv_content, line_meta) = eg.get_line(self.tv.get_scroll(), inp, is_exact);
                self.terminal.draw(|f| {
                    let (txt_clr, inp_clr, chat_clr_, chat_inp_clr) = self.focus.get_colors();
                    // 左下输入框区
                    let input_box = Paragraph::new(Text::raw(self.fuzzy_inp.get_inp()))
                        .block(
                            Block::default()
                                .title("search")
                                .borders(Borders::TOP | Borders::LEFT),
                        )
                        .style(Style::default().fg(inp_clr)); // 设置输入框样式
                    f.render_widget(input_box, self.fuzzy_inp.get_rect());
                    let block = Block::default().borders(Borders::LEFT);

                    if let Some(c) = &tv_content {
                        let (navi, visible_content) = get_content(
                            c,
                            &line_meta,
                            self.navi.get_cur_line(),
                            &self.navi.select_line,
                            self.tv.get_height(),
                        );
                        let text_para = Paragraph::new(visible_content)
                            .block(block)
                            .style(Style::default().fg(txt_clr));
                        f.render_widget(text_para, self.tv.get_rect());
                        let nav_paragraph = Paragraph::new(navi);
                        f.render_widget(nav_paragraph, self.navi.get_rect());
                    } else {
                        let text_para = Paragraph::new("")
                            .block(block)
                            .style(Style::default().fg(txt_clr));
                        f.render_widget(text_para, self.tv.get_rect());
                        let nav_paragraph = Paragraph::new("");
                        f.render_widget(nav_paragraph, self.navi.get_rect());
                    }

                    match chat_type {
                        ChatType::ChatTv => {
                            let (chat_content, chat_line_meta) =
                                chat_eg.get_line(self.chat_tv.get_scroll(), "", is_exact);
                            if let Some(c) = &chat_content {
                                let chat_content =
                                    get_chat_content(&c, &chat_line_meta, &chat_item[chat_index]);
                                let chat_tv = Paragraph::new(chat_content)
                                    .block(Block::default().borders(Borders::LEFT | Borders::RIGHT))
                                    .style(Style::default().fg(chat_clr_));
                                f.render_widget(chat_tv, self.chat_tv.get_rect());
                            } else {
                                let chat_tv = Paragraph::new("")
                                    .block(Block::default().borders(Borders::LEFT | Borders::RIGHT))
                                    .style(Style::default().fg(chat_clr_));
                                f.render_widget(chat_tv, self.chat_tv.get_rect());
                            }
                        }
                        ChatType::Promt => {}
                        ChatType::Pattern => {}
                    }
                    // 右侧部分可以显示空白或其他内容
                    // let block = Block::default().borders(Borders::ALL).title("LLM Chat");
                    let input_box = Paragraph::new(Text::raw(self.chat_inp.get_inp()))
                        .block(
                            Block::default()
                                .title("prompt")
                                .borders(Borders::TOP | Borders::LEFT | Borders::RIGHT),
                        )
                        .style(Style::default().fg(chat_inp_clr)); // 设置输入框样式
                    f.render_widget(input_box, self.chat_inp.get_rect());

                    match self.focus.current() {
                        FocusType::TxtFuzzy => {
                            // 将光标移动到输入框中合适的位置
                            let inp_len = self.fuzzy_inp.get_inp().width();
                            let x = if inp_len == 0 {
                                self.fuzzy_inp.get_rect().x + 2
                            } else {
                                self.fuzzy_inp.get_rect().x + inp_len as u16 + 1
                            };

                            let y = self.fuzzy_inp.get_rect().y + 1; // 输入框的 Y 起点
                            f.set_cursor_position(Position { x, y });
                        }
                        FocusType::Chat => {
                            // 将光标移动到输入框中合适的位置
                            let inp_len = self.chat_inp.get_inp().width();
                            let x = if inp_len == 0 {
                                self.chat_inp.get_rect().x + 2
                            } else {
                                self.chat_inp.get_rect().x + inp_len as u16 + 1
                            };
                            let y = self.chat_inp.get_rect().y + 1; // 输入框的 Y 起点
                            f.set_cursor_position(Position { x, y });
                        }
                        _ => {}
                    };
                })?;
                line_meta
            };

            loop {
                tokio::select! {
                    Some(msg) = llm_res_rx.recv() => {
                        let chat_item_start = chat_eg.get_line_count().max(1);
                        //self.chat_tv.set_scroll(chat_item_start);
                        chat_eg.push_str(&msg);
                        let chat_item_end = chat_eg.get_line_count().max(1);
                        chat_item
                            .push(ChatItemIndex(chat_item_start, chat_item_end));
                        chat_eg.push_str("\n\n");
                        break;
                    }
                    _ = tokio::time::sleep(Duration::from_millis(25)) => {
                    }
                }
                // 监听键盘输入
                if event::poll(Duration::from_millis(50)).unwrap() {
                    if let event::Event::Key(KeyEvent {
                        code, modifiers, ..
                    }) = event::read()?
                    {
                        match (code, modifiers) {
                            (KeyCode::Esc, _) => {
                                self.fuzzy_inp.clear();
                                self.chat_inp.clear();
                                self.navi.select_line = None;
                                break;
                            }
                            (KeyCode::Tab, _) => {
                                // 按下 Tab 键，切换焦点
                                self.focus.next();
                                break;
                            }
                            (KeyCode::Char('s'), KeyModifiers::CONTROL) => {
                                let chat_inp = self.chat_inp.get_inp();
                                if chat_inp.len() == 0 {
                                    break;
                                }
                                let mut message = String::new();
                                if let Some((start_line, end_line)) = self.navi.select_line {
                                    if let (Some(line), _) = eg.get_start_end(start_line, end_line)
                                    {
                                        for l in line.iter() {
                                            message.push_str(l);
                                        }
                                    }
                                }
                                message.push_str("\n");
                                message.push_str(chat_inp);
                                if let Ok(searcher) = self.vb.searcher().await {
                                    let prompt_field_id =
                                        self.vb.get_schema().get_field("prompt").unwrap();
                                    let _id_field_id =
                                        self.vb.get_schema().get_field("_id").unwrap();
                                    let answer_field_id =
                                        self.vb.get_schema().get_field("answer").unwrap();
                                    let embeddings =
                                        self.embed_model.embed(vec![&message], None).unwrap();
                                    for (_, v) in embeddings.iter().enumerate() {
                                        let tensor = Tensor::arr_slice(v);
                                        for ns in searcher.query(&tensor, 1, None)? {
                                            let v = searcher.vector(&ns)?;
                                            let prompt = v
                                                .doc()
                                                .get_field_value(prompt_field_id)
                                                .value()
                                                .str();
                                            let answer = v
                                                .doc()
                                                .get_field_value(answer_field_id)
                                                .value()
                                                .str();
                                            let chat_item_start = chat_eg.get_line_count().max(1);
                                            self.chat_tv.set_scroll(chat_item_start);
                                            chat_eg.push_str(&format!("----------------------------\n{}\n----------------------------\n",prompt));
                                            chat_eg.push_str(answer);
                                            chat_eg.push_str("\n\n");
                                            let chat_item_end = chat_eg.get_line_count().max(1) - 1;
                                            chat_item.push(ChatItemIndex(
                                                chat_item_start,
                                                chat_item_end,
                                            ));
                                            self.chat_inp.clear();
                                            chat_index = chat_item.len() - 1;
                                        }
                                    }
                                }
                                break;
                            }
                            (KeyCode::Char('x'), KeyModifiers::CONTROL) => {
                                crossterm::terminal::disable_raw_mode()?;
                                self.terminal.clear()?;
                                execute!(
                                    self.terminal.backend_mut(),
                                    LeaveAlternateScreen // 离开备用屏幕
                                )?;
                                if chat_item.len() > 0 {
                                    // 在下一行打印退出消息

                                    self.terminal
                                        .backend_mut()
                                        .execute(cursor::MoveToNextLine(1))?;
                                    let item = &chat_item[chat_index];
                                    if let (Some(msg), _) =
                                        chat_eg.get_start_end(item.start(), item.end())
                                    {
                                        println!("{}", msg.join(""));
                                    }
                                }
                                self.terminal.backend_mut().flush()?;
                                exit(0);
                                //break;
                            }
                            (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                                crossterm::terminal::disable_raw_mode()?;
                                execute!(
                                    self.terminal.backend_mut(),
                                    LeaveAlternateScreen // 离开备用屏幕
                                )?;
                                self.terminal.show_cursor()?; // 确保光标可见
                                exit(0);
                            }
                            (KeyCode::Enter, _) => match self.focus.current() {
                                FocusType::TxtFuzzy => {
                                    if let Some((_, _)) = self.navi.select_line {
                                        self.focus.next();
                                        self.focus.next();
                                        self.focus.next();
                                    } else {
                                        self.fuzzy_inp.clear();
                                        let cur_line = self.navi.get_cur_line();
                                        if cur_line < line_meta.len() {
                                            self.tv.set_scroll(line_meta[cur_line].get_line_num());
                                            self.navi.to_min_line();
                                        }
                                    }
                                    break;
                                }
                                FocusType::Chat => {
                                    let chat_inp = self.chat_inp.get_inp();
                                    if chat_inp.len() == 0 {
                                        break;
                                    }
                                    let mut message = String::new();
                                    if let Some((start_line, end_line)) = self.navi.select_line {
                                        if let (Some(line), _) =
                                            eg.get_start_end(start_line, end_line)
                                        {
                                            for l in line.iter() {
                                                message.push_str(l);
                                            }
                                        }
                                    }
                                    message.push_str("\n");
                                    message.push_str(chat_inp);

                                    if message.trim().len() > 0 {
                                        let chat_item_start = chat_eg.get_line_count().max(1);
                                        self.chat_tv.set_scroll(chat_item_start);
                                        chat_eg.push_str(&format!("----------------------------\n{}\n----------------------------\n",message));
                                        let chat_item_end = chat_eg.get_line_count().max(1) - 1;
                                        chat_item
                                            .push(ChatItemIndex(chat_item_start, chat_item_end));
                                        let _ = self.prompt_tx.send(message.to_string()).await;
                                        self.chat_inp.clear();
                                        chat_index = chat_item.len() - 1;
                                        chat_type = ChatType::ChatTv;
                                        break;
                                    }
                                }
                                _ => {}
                            },
                            (KeyCode::Down, KeyModifiers::SHIFT) => {
                                // 下一页
                                match self.focus.current() {
                                    FocusType::TxtFuzzy => {
                                        let sel_line = if self.navi.is_bottom() {
                                            self.tv.down_line(eg.get_max_scroll_num());
                                            self.tv.scroll + self.tv.get_height()
                                        } else {
                                            self.navi.down_line();
                                            let cur_line = self.navi.get_cur_line();
                                            if cur_line >= line_meta.len() {
                                                break;
                                            }
                                            let sel_line = line_meta[cur_line].get_line_num();
                                            sel_line
                                        };
                                        match self.navi.select_line {
                                            Some((st, en)) => {
                                                if sel_line == en {
                                                    if let Some(max_num) = eg.get_max_scroll_num() {
                                                        if sel_line == max_num {
                                                            self.navi.select_line =
                                                                Some((st, sel_line));
                                                        }
                                                    } else {
                                                        self.navi.select_line =
                                                            Some((sel_line, sel_line));
                                                    }
                                                } else if sel_line > st && sel_line > en {
                                                    self.navi.select_line = Some((st, sel_line));
                                                } else if sel_line > st && sel_line < en {
                                                    self.navi.select_line = Some((sel_line, en));
                                                }
                                            }
                                            None => {
                                                self.navi.select_line =
                                                    Some((sel_line - 1, sel_line - 1));
                                            }
                                        }

                                        break;
                                    }
                                    FocusType::Chat => {
                                        self.chat_tv.down_line(chat_eg.get_max_scroll_num());
                                        break;
                                    }
                                    _ => {}
                                }
                                break;
                            }
                            (KeyCode::Up, KeyModifiers::SHIFT) => {
                                match self.focus.current() {
                                    FocusType::TxtFuzzy => {
                                        let sel_line = if self.navi.is_top() {
                                            self.tv.up_line();
                                            (self.tv.scroll - 1).max(1)
                                        } else {
                                            self.navi.up_line();

                                            let cur_line = self.navi.get_cur_line();
                                            if cur_line >= line_meta.len() {
                                                break;
                                            }
                                            let sel_line = line_meta[cur_line].get_line_num();
                                            sel_line
                                        };

                                        match self.navi.select_line {
                                            Some((st, en)) => {
                                                if sel_line == st {
                                                    if sel_line == 1 {
                                                        self.navi.select_line = Some((st, en));
                                                    } else {
                                                        self.navi.select_line =
                                                            Some((sel_line, sel_line));
                                                    }
                                                } else if sel_line > st && sel_line < en {
                                                    self.navi.select_line = Some((st, sel_line));
                                                } else if sel_line < st {
                                                    self.navi.select_line = Some((sel_line, en));
                                                }
                                            }
                                            None => {
                                                self.navi.select_line =
                                                    Some((sel_line + 1, sel_line + 1));
                                            }
                                        }
                                    }

                                    FocusType::Chat => {
                                        // self.chat_tv.up_line();
                                    }
                                    _ => {}
                                }
                                break;
                            }
                            (KeyCode::Down, KeyModifiers::CONTROL) => {
                                // 下一页
                                match self.focus.current() {
                                    FocusType::TxtFuzzy => {
                                        self.tv.down_page(eg.get_max_scroll_num());
                                    }
                                    FocusType::Chat => {}
                                    _ => {}
                                }
                                break;
                            }
                            (KeyCode::Up, KeyModifiers::CONTROL) => {
                                match self.focus.current() {
                                    FocusType::TxtFuzzy => {
                                        self.tv.up_page();
                                    }
                                    FocusType::Chat => {
                                        //  self.chat_tv.up_page();
                                    }
                                    _ => {}
                                }

                                break;
                            }
                            (KeyCode::Up, _) => {
                                // 向上滚动
                                match self.focus.current() {
                                    FocusType::TxtFuzzy => {
                                        if self.navi.is_top() {
                                            self.tv.up_line();
                                        } else {
                                            self.navi.up_line();
                                        }
                                        break;
                                    }
                                    FocusType::Chat => {
                                        // self.chat_tv.up_line();
                                        let chat_item_index = &chat_item[chat_index];
                                        let start = chat_item_index.start();
                                        let pre_scorll = chat_scorll
                                            .saturating_sub(self.chat_tv.get_height())
                                            .max(1);
                                        if pre_scorll >= start {
                                            chat_scorll = pre_scorll;
                                            self.chat_tv.set_scroll(chat_scorll);
                                        } else if chat_scorll <= start {
                                            chat_index = chat_index.saturating_sub(1);
                                            let pre_chat_item_index = &chat_item[chat_index];
                                            let pre_start = pre_chat_item_index.start();
                                            chat_scorll = pre_start;
                                            self.chat_tv.set_scroll(chat_scorll);
                                        } else if pre_scorll < start {
                                            chat_scorll = start;
                                            self.chat_tv.set_scroll(chat_scorll);
                                        }

                                        break;
                                    }
                                    _ => {}
                                }
                            }
                            (KeyCode::Down, _) => {
                                // 向下滚动
                                match self.focus.current() {
                                    FocusType::TxtFuzzy => {
                                        if self.navi.is_bottom() {
                                            self.tv.down_line(eg.get_max_scroll_num());
                                        } else {
                                            self.navi.down_line();
                                        }
                                        break;
                                    }
                                    FocusType::Chat => {
                                        match chat_type {
                                            ChatType::ChatTv => {
                                                // 下一个item
                                                let chat_item_index = &chat_item[chat_index];
                                                let start = chat_item_index.start();
                                                let end = chat_item_index.end();
                                                if chat_scorll < start {
                                                    chat_scorll = start;
                                                    self.chat_tv.set_scroll(chat_scorll);
                                                } else if chat_scorll + self.chat_tv.get_height()
                                                    <= end
                                                {
                                                    chat_scorll =
                                                        chat_scorll + self.chat_tv.get_height();
                                                    self.chat_tv.set_scroll(chat_scorll);
                                                } else if chat_scorll + self.chat_tv.get_height()
                                                    > end
                                                {
                                                    if chat_index + 1 <= chat_item.len() - 1 {
                                                        chat_index += 1;
                                                        let next_chat_item_index =
                                                            &chat_item[chat_index];
                                                        let next_start =
                                                            next_chat_item_index.start();
                                                        chat_scorll = next_start;
                                                        self.chat_tv.set_scroll(chat_scorll);
                                                    }
                                                }
                                            }
                                            ChatType::Promt => {}
                                            ChatType::Pattern => {}
                                        }

                                        break;
                                    }
                                    _ => {}
                                }
                            }
                            (KeyCode::Char(c), _) => {
                                match self.focus.current() {
                                    FocusType::TxtFuzzy => {
                                        self.fuzzy_inp.push(c); // 添加字符到输入缓冲区
                                        self.tv.set_scroll(1);
                                        break;
                                    }
                                    FocusType::Chat => {
                                        self.chat_inp.push(c); // 添加字符到输入缓冲区
                                        break;
                                    }
                                    _ => {}
                                }

                                break;
                            }
                            (KeyCode::Backspace, _) => {
                                match self.focus.current() {
                                    FocusType::TxtFuzzy => {
                                        self.fuzzy_inp.pop();
                                        self.tv.set_scroll(1);
                                        break;
                                    }
                                    FocusType::Chat => {
                                        self.chat_inp.pop();
                                        break;
                                    }
                                    _ => {}
                                }

                                break;
                            }

                            _ => {}
                        }
                    }
                }
            }
        }
    }
}

struct Navigation {
    min_line: usize,
    max_line: usize,
    cur_line: usize,
    select_line: Option<(usize, usize)>,
    rect: Rect,
}

impl Navigation {
    fn get_rect(&self) -> Rect {
        self.rect
    }

    fn is_top(&self) -> bool {
        self.cur_line == self.min_line
    }

    fn is_bottom(&self) -> bool {
        self.cur_line == self.max_line
    }

    fn down_line(&mut self) {
        if self.cur_line < self.max_line {
            self.cur_line += 1;
        }
    }

    fn up_line(&mut self) {
        if self.cur_line > self.min_line {
            self.cur_line -= 1;
        }
    }

    fn get_cur_line(&self) -> usize {
        self.cur_line
    }

    fn to_min_line(&mut self) {
        self.cur_line = self.min_line
    }

    fn to_max_line(&mut self) {
        self.cur_line = self.max_line
    }

    fn set_cur_line(&mut self, cur_line: usize) {
        self.cur_line = cur_line
    }
}

struct TextView {
    height: usize,
    width: usize,
    scroll: usize, //当前页 第一行 行数
    rect: Rect,
}

impl TextView {
    fn get_rect(&self) -> Rect {
        self.rect
    }

    fn get_height(&self) -> usize {
        self.height
    }

    fn get_width(&self) -> usize {
        self.width
    }

    fn get_scroll(&self) -> usize {
        self.scroll
    }

    fn set_scroll(&mut self, scroll: usize) {
        self.scroll = scroll
    }

    fn up_line(&mut self) {
        self.scroll = (self.scroll - 1).max(1);
    }

    fn down_line(&mut self, max_num: Option<usize>) {
        if let Some(max_scroll_num) = max_num {
            if self.scroll <= max_scroll_num {
                self.scroll += 1;
            }
        } else {
            self.scroll += 1;
        }
    }

    fn up_page(&mut self) {
        if self.scroll > self.height {
            self.scroll = (self.scroll - self.height).max(1);
        } else {
            self.scroll = 1;
        }
    }

    fn down_page(&mut self, max_num: Option<usize>) {
        if let Some(max_scroll_num) = max_num {
            self.scroll = ((self.scroll + self.height).min(max_scroll_num)).max(1)
        } else {
            self.scroll += self.height;
        }
    }
}

struct FuzzyInput {
    input: String,
    rect: Rect,
}

impl FuzzyInput {
    fn get_rect(&self) -> Rect {
        self.rect
    }

    fn clear(&mut self) {
        self.input.clear();
    }

    fn push(&mut self, c: char) {
        self.input.push(c);
    }

    fn pop(&mut self) {
        self.input.pop();
    }

    fn get_inp(&self) -> &str {
        &self.input
    }

    fn get_inp_exact(&self) -> (&str, bool) {
        return if let Some(first_char) = &self.input.chars().next() {
            if *first_char == '/' {
                (&self.input[1..].trim(), false)
            } else {
                (&self.input.trim(), true)
            }
        } else {
            (&self.input.trim(), true)
        };
    }
}

struct ChatText {
    height: usize,
    width: usize,
    scroll: usize,
    rect: Rect,
}

impl ChatText {
    fn get_rect(&self) -> Rect {
        self.rect
    }

    fn get_height(&self) -> usize {
        self.height
    }

    fn get_width(&self) -> usize {
        self.width
    }
    fn get_scroll(&self) -> usize {
        self.scroll
    }

    fn set_scroll(&mut self, scroll: usize) {
        self.scroll = scroll
    }

    fn up_line(&mut self) {
        self.scroll = (self.scroll - 1).max(1);
    }

    fn down_line(&mut self, max_num: Option<usize>) {
        if let Some(max_scroll_num) = max_num {
            if self.scroll <= max_scroll_num {
                self.scroll += 1;
            }
        } else {
            self.scroll += 1;
        }
    }

    fn up_page(&mut self) {
        if self.scroll > self.height {
            self.scroll = (self.scroll - self.height).max(1);
        } else {
            self.scroll = 1;
        }
    }

    fn down_page(&mut self, max_num: Option<usize>) {
        if let Some(max_scroll_num) = max_num {
            self.scroll = (self.scroll + self.height).min(max_scroll_num)
        } else {
            self.scroll += self.height - 1;
        }
    }
}

struct ChatInput {
    input: String,
    rect: Rect,
}

impl ChatInput {
    fn get_rect(&self) -> Rect {
        self.rect
    }
    fn clear(&mut self) {
        self.input.clear();
    }

    fn push(&mut self, c: char) {
        self.input.push(c);
    }

    fn pop(&mut self) {
        self.input.pop();
    }

    fn get_inp(&self) -> &str {
        &self.input
    }
}

fn get_chat_content<'a>(
    content: &'a Vec<&'a str>,
    line_meta: &Vec<LineMeta>,
    chat_item: &ChatItemIndex,
) -> (Text<'a>) {
    assert!(content.len() == line_meta.len());
    let mut lines = Vec::new();
    for (i, text) in content.into_iter().enumerate() {
        // let mut spans = Vec::new();
        let line_num = line_meta[i].get_line_num();
        if line_num >= chat_item.start() && line_num <= chat_item.end() {
            lines.push(Line::from(Span::styled(
                *text,
                Style::default().fg(Color::Green),
            )));
        } else {
            lines.push(Line::from(*text));
        }
        // let mf = line_meta[i].get_match(); //fuzzy_search(input, text, false);
        // if let Some(m) = mf {
        //     match m {
        //         Match::Char(_) => {
        //             todo!()
        //         }
        //         Match::Byte(v) => {
        //             let mut current_idx = 0;
        //             for bm in v.into_iter() {
        //                 if current_idx < bm.start && bm.start <= text.len() {
        //                     spans.push(Span::raw(&text[current_idx..bm.start]));
        //                 }
        //                 // 添加高亮文本
        //                 if bm.start < text.len() && bm.end <= text.len() {
        //                     spans.push(Span::styled(
        //                         &text[bm.start..bm.end],
        //                         Style::default().bg(Color::Green),
        //                     ));
        //                 }
        //                 // 更新当前索引为高亮区间的结束位置
        //                 current_idx = bm.end;
        //             }
        //             // 添加剩余的文本（如果有）
        //             if current_idx < text.len() {
        //                 spans.push(Span::raw(&text[current_idx..]));
        //             }
        //         }
        //     }
        // }

        // if spans.len() > 0 {
        //     lines.push(Line::from(spans));
        // } else {
        //     lines.push(Line::from(*text));
        // }
    }
    let text = Text::from(lines);
    text
}

fn get_content<'a>(
    content: &'a Vec<&str>,
    line_meta: &Vec<LineMeta>,
    cur_line: usize,
    select_line: &Option<(usize, usize)>,
    height: usize,
) -> (Text<'a>, Text<'a>) {
    assert!(content.len() == line_meta.len());
    let mut lines = Vec::new();
    for (i, text) in content.into_iter().enumerate() {
        let mut spans = Vec::new();
        let mf = line_meta[i].get_match(); //fuzzy_search(input, text, false);
        if let Some(m) = mf {
            match m {
                Match::Char(_) => {
                    todo!()
                }
                Match::Byte(v) => {
                    let mut current_idx = 0;
                    for bm in v.into_iter() {
                        if current_idx < bm.start && bm.start <= text.len() {
                            spans.push(Span::raw(&text[current_idx..bm.start]));
                        }
                        // 添加高亮文本
                        if bm.start < text.len() && bm.end <= text.len() {
                            spans.push(Span::styled(
                                &text[bm.start..bm.end],
                                Style::default().bg(Color::Green),
                            ));
                        }
                        // 更新当前索引为高亮区间的结束位置
                        current_idx = bm.end;
                    }
                    // 添加剩余的文本（如果有）
                    if current_idx < text.len() {
                        spans.push(Span::raw(&text[current_idx..]));
                    }
                }
            }
        }
        if let Some((st, en)) = select_line {
            if (line_meta[i].get_line_num() >= *st && line_meta[i].get_line_num() <= *en)
                || i == cur_line
            {
                lines.push(Line::from(Span::styled(
                    *text,
                    Style::default().bg(Color::LightRed), // 设置背景颜色为红色
                )));
            } else {
                if spans.len() > 0 {
                    lines.push(Line::from(spans));
                } else {
                    lines.push(Line::from(*text));
                }
            }
        } else {
            if i == cur_line {
                lines.push(Line::from(Span::styled(
                    *text,
                    Style::default().bg(Color::LightRed), // 设置背景颜色为蓝色
                )));
            } else {
                if spans.len() > 0 {
                    lines.push(Line::from(spans));
                } else {
                    lines.push(Line::from(*text));
                }
            }
        }
    }

    let nav_text = Text::from(
        (0..height)
            .enumerate()
            .map(|(i, _)| {
                if i == cur_line {
                    Line::from(Span::styled(">", Style::default().fg(Color::LightRed)))
                // 高亮当前行
                } else {
                    Line::from(" ") // 非当前行为空白
                }
            })
            .collect::<Vec<Line>>(),
    );

    let text = Text::from(lines);
    (nav_text, text)
}

// 获取要显示的内容（根据终端高度和偏移量）

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn test_mmap() -> io::Result<()> {
        // let file_path = "/root/start_vpn.sh";
        // let mmap = map_file(file_path)?;
        // let (navi, visible_content, length) = get_visible_content(&mmap, 0, 30, 5, "");
        // println!("{},{}", visible_content, length);
        // Ok(())
        Ok(())
    }
}
