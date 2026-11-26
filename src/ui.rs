use std::{cell::{Cell, RefCell}, collections::{BTreeSet, HashMap}, path::{Path,PathBuf}, rc::Rc, sync::{Arc,atomic::{AtomicBool,Ordering}}, time::Duration};
use windows_core::{Interface,HSTRING,IInspectable};
use naui_windows::{Ui,Widget};
use naui_winui3::Microsoft::UI::Xaml::{self as x, Controls::*, Markup::XamlReader, FrameworkElement,UIElement,RoutedEventHandler};
use crate::backend::{self,Backend,Command,Snapshot,WindowRow};

#[derive(Clone)] struct Native(UIElement);
impl Widget for Native { fn native_element(&self)->UIElement {self.0.clone()} fn boxed_clone(&self)->Box<dyn Widget>{Box::new(self.clone())} }
fn element<T:Interface>(root:&FrameworkElement,name:&str)->windows_core::Result<T> {root.FindName(&HSTRING::from(name))?.cast()}
fn click(root:&FrameworkElement,name:&str,f:impl Fn()+'static)->windows_core::Result<()> {element::<Button>(root,name)?.Click(&routed(move||{f();Ok(())}))?;Ok(())}
thread_local! { static CALLBACKS: RefCell<HashMap<u64,Box<dyn FnMut()->windows_core::Result<()>>>> = RefCell::new(HashMap::new()); }
static CALLBACK_ID: std::sync::atomic::AtomicU64=std::sync::atomic::AtomicU64::new(1);
fn register(f:impl FnMut()->windows_core::Result<()>+'static)->u64{let id=CALLBACK_ID.fetch_add(1,Ordering::Relaxed);CALLBACKS.with(|m|{m.borrow_mut().insert(id,Box::new(f));});id}
fn invoke(id:u64)->windows_core::Result<()>{let mut callback=CALLBACKS.with(|m|m.borrow_mut().remove(&id));if let Some(ref mut f)=callback{let result=f();CALLBACKS.with(|m|{m.borrow_mut().insert(id,callback.take().unwrap());});result}else{Ok(())}}
fn routed(f:impl FnMut()->windows_core::Result<()>+'static)->RoutedEventHandler{let id=register(f);RoutedEventHandler::new(move|_,_|invoke(id))}
fn text_changed(f:impl FnMut()->windows_core::Result<()>+'static)->TextChangedEventHandler{let id=register(f);TextChangedEventHandler::new(move|_,_|invoke(id))}
fn selection_changed(f:impl FnMut()->windows_core::Result<()>+'static)->SelectionChangedEventHandler{let id=register(f);SelectionChangedEventHandler::new(move|_,_|invoke(id))}
pub fn clear_handlers(){let old=CALLBACKS.with(|m|std::mem::take(&mut *m.borrow_mut()));drop(old);}
fn log(path:&Path,msg:&str){use std::io::Write; if let Ok(mut f)=std::fs::OpenOptions::new().create(true).append(true).open(path.join("ui.log")){let _=writeln!(f,"{msg}");}}

struct RowUi { panel:FrameworkElement, check:CheckBox, title:TextBlock, detail:TextBlock, state:TextBlock, last:(String,String,String,bool,bool), handlers:Vec<u64> }
impl Drop for RowUi {fn drop(&mut self){let removed=CALLBACKS.try_with(|m|{let mut m=m.borrow_mut();self.handlers.iter().filter_map(|id|m.remove(id)).collect::<Vec<_>>()});drop(removed);}}
struct View { root:FrameworkElement,rows:StackPanel,registry:HashMap<isize,RowUi>,order:Vec<isize>,checked:Rc<RefCell<BTreeSet<isize>>>,tx:std::sync::mpsc::Sender<Command>,snapshot:Snapshot,query:String,sort:i32,monitor_ids:Vec<String> }
impl View {
 fn selected(&self)->Vec<isize>{self.order.iter().copied().filter(|id|self.checked.borrow().contains(id)).collect()}
 fn refresh(&mut self,snapshot:Snapshot)->windows_core::Result<()> {
  self.snapshot=snapshot;
  let valid:BTreeSet<_>=self.snapshot.windows.iter().map(|r|r.id).collect();
  self.checked.borrow_mut().retain(|id|valid.contains(id));
  if self.monitor_ids!=self.snapshot.monitors.iter().map(|m|m.id.clone()).collect::<Vec<_>>() {
   self.reset_order();
   self.monitor_ids=self.snapshot.monitors.iter().map(|m|m.id.clone()).collect();
   for name in ["Target","MouseTarget"] { let c:ComboBox=element(&self.root,name)?; let index=c.SelectedIndex().unwrap_or(0).max(0);let items=c.Items()?; items.Clear()?;for m in &self.snapshot.monitors {let item=ComboBoxItem::new()?; let boxed=windows::Foundation::PropertyValue::CreateString(&HSTRING::from(&m.label))?;item.SetContent(&boxed)?;items.Append(&item)?;} if !self.monitor_ids.is_empty(){c.SetSelectedIndex(index.min(self.monitor_ids.len() as i32-1))?;} }
  }
  let q=self.query.to_lowercase();
  let mut rows=self.snapshot.windows.iter().filter(|r| q.is_empty()||r.title.to_lowercase().contains(&q)||r.process.to_lowercase().contains(&q)).cloned().collect::<Vec<_>>();
  rows.sort_by_key(|r| match self.sort {1=>(r.title.to_lowercase(),r.id),2=>(r.monitor.clone(),r.id),3=>(r.state.clone(),r.id),_=>(r.process.to_lowercase(),r.id)});
  let ids:Vec<_>=rows.iter().map(|r|r.id).collect();
  // Keep existing rows in place on state changes; sorting changes explicitly rebuild order.
  let present:BTreeSet<_>=ids.iter().copied().collect();
  let children=self.rows.Children()?;
  for id in self.order.clone() {if !present.contains(&id) {if let Some(row)=self.registry.remove(&id){let obj:UIElement=row.panel.cast()?;let mut index=0;if children.IndexOf(&obj,&mut index)?{children.RemoveAt(index)?;}}}}
  self.order.retain(|id|present.contains(id));
  for r in &rows {
   if !self.registry.contains_key(&r.id) {let row=self.make_row(r)?;children.Append(&row.panel)?;self.registry.insert(r.id,row);self.order.push(r.id);}
   if r.blacklisted {self.checked.borrow_mut().remove(&r.id);}
   let row=self.registry.get_mut(&r.id).unwrap();
   let checked=self.checked.borrow().contains(&r.id);
   let state=format!("{}\n{}{}",r.monitor,r.state,if r.blacklisted{" · 黑名单"}else if r.locked{" · 已锁定"}else{""});
   let next=(r.title.clone(),r.process.clone(),state,checked,r.blacklisted);
   if row.last.0!=next.0 {row.title.SetText(&HSTRING::from(&next.0))?;}
   if row.last.1!=next.1 {row.detail.SetText(&HSTRING::from(&next.1))?;}
   if row.last.2!=next.2 {row.state.SetText(&HSTRING::from(&next.2))?;}
   if row.last.4!=next.4 {row.check.SetIsEnabled(!r.blacklisted)?;}
   if row.last.3!=checked {let boxed:windows::Foundation::IReference<bool>=windows::Foundation::PropertyValue::CreateBoolean(checked)?.cast()?;row.check.SetIsChecked(&boxed)?;}
   row.last=next;
  }
  let has_selection=!self.selected().is_empty();
  for name in ["Move","Lock","Unlock","Restore","Cancel"] {element::<Button>(&self.root,name)?.SetIsEnabled(has_selection)?;}
  element::<TextBlock>(&self.root,"Count")?.SetText(&HSTRING::from(format!("{} 个窗口 · 已勾选 {} 个",rows.len(),self.checked.borrow().len())))?;
  element::<TextBlock>(&self.root,"Status")?.SetText(&HSTRING::from(&self.snapshot.status))?;
  element::<TextBlock>(&self.root,"MouseStatus")?.SetText(&HSTRING::from(match &self.snapshot.mouse_target {Some(m)=>format!("鼠标：锁定到 {m}"),None=>"鼠标：未锁定".into()}))?;
  element::<TextBlock>(&self.root,"BlacklistRules")?.SetText(&HSTRING::from(if self.snapshot.blacklist.is_empty(){"暂无黑名单规则".into()}else{format!("现有规则：{}",self.snapshot.blacklist.join("、"))}))?;
  self.layout()?;
  element::<TextBlock>(&self.root,"Empty")?.SetVisibility(if rows.is_empty(){x::Visibility::Visible}else{x::Visibility::Collapsed})?;
  Ok(())
 }
 fn layout(&self)->windows_core::Result<()> {
  let width=self.root.ActualWidth()?;
  for (name,minimum) in [("TopActions",550.),("SelectionActions",465.),("WindowActions",480.),("SettingsActions",580.)] {
   element::<StackPanel>(&self.root,name)?.SetOrientation(if width>=minimum{Orientation::Horizontal}else{Orientation::Vertical})?;
  }
  for row in self.registry.values() {
   let compact=width<760.;
   let grid:Grid=row.panel.cast()?;
   grid.ColumnDefinitions()?.GetAt(2)?.SetWidth(x::GridLength{Value:if compact{0.}else{230.},GridUnitType:x::GridUnitType::Pixel})?;
   Grid::SetColumn(&row.state,if compact{1}else{2})?;
   Grid::SetRow(&row.state,if compact{1}else{0})?;
  }
  Ok(())
 }
 fn reset_order(&mut self){if let Ok(c)=self.rows.Children(){let _=c.Clear();}self.registry.clear();self.order.clear();}
 fn make_row(&self,r:&WindowRow)->windows_core::Result<RowUi>{
  let first=CALLBACK_ID.load(Ordering::Relaxed);
  let panel:FrameworkElement=XamlReader::Load(&HSTRING::from(r#"<Grid xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation" xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml" Padding="4,8" ColumnSpacing="12" MinHeight="64"><Grid.RowDefinitions><RowDefinition Height="Auto"/><RowDefinition Height="Auto"/></Grid.RowDefinitions><Grid.ColumnDefinitions><ColumnDefinition Width="32"/><ColumnDefinition Width="*"/><ColumnDefinition Width="230"/></Grid.ColumnDefinitions><CheckBox x:Name="Check" VerticalAlignment="Center"/><StackPanel Grid.Column="1" Spacing="2"><TextBlock x:Name="Title" FontWeight="SemiBold" TextTrimming="CharacterEllipsis"/><TextBlock x:Name="Detail" Opacity="0.65" FontSize="12" TextTrimming="CharacterEllipsis"/></StackPanel><TextBlock x:Name="State" Grid.Column="2" VerticalAlignment="Center" TextWrapping="Wrap" Opacity="0.8"/></Grid>"#))?.cast()?;
  let check:CheckBox=element(&panel,"Check")?;let id=r.id;let selected=self.checked.clone();
  check.Click(&routed(move ||{let mut set=selected.borrow_mut();if !set.remove(&id){set.insert(id);}Ok(())}))?;
  let fly=MenuFlyout::new()?;
  for (label,kind) in [("解除最小化",0),("锁定到当前显示器",1),("解锁",2),("置顶 / 取消置顶",3),("加入黑名单",4)] {
   let item=MenuFlyoutItem::new()?;item.SetText(&HSTRING::from(label))?;let tx=self.tx.clone();let proc=r.process.clone();let checked=self.checked.clone();
   item.Click(&routed(move ||{let ids=if checked.borrow().is_empty(){vec![id]}else{checked.borrow().iter().copied().collect()};let cmd=match kind{0=>Command::Restore(ids),1=>Command::Lock(ids),2=>Command::Unlock(ids),3=>Command::ToggleTopmost(id),_=>Command::AddBlacklist(proc.clone())};let _=tx.send(cmd);Ok(())}))?;fly.Items()?.Append(&item)?;
  }
  for m in &self.snapshot.monitors {let item=MenuFlyoutItem::new()?;item.SetText(&HSTRING::from(format!("移动到 {}",m.label)))?;let tx=self.tx.clone();let target=m.id.clone();let checked=self.checked.clone();item.Click(&routed(move ||{let ids=if checked.borrow().is_empty(){vec![id]}else{checked.borrow().iter().copied().collect()};let _=tx.send(Command::Move(ids,target.clone()));Ok(())}))?;fly.Items()?.Append(&item)?;}
  panel.SetContextFlyout(&fly)?;
  Ok(RowUi{check,title:element(&panel,"Title")?,detail:element(&panel,"Detail")?,state:element(&panel,"State")?,last:(String::new(),String::new(),String::new(),false,false),handlers:(first..CALLBACK_ID.load(Ordering::Relaxed)).collect(),panel})
 }
}
pub fn build(ui:&Ui,backend:Backend,smoke:bool,readonly:bool,data:&Path)->windows_core::Result<()> {
 let title = concat!("WindowLockMaster Rust ", env!("CARGO_PKG_VERSION"));
 let window=ui.window(title,400.,300.).map_err(|e|windows_core::Error::new(windows::Win32::Foundation::E_FAIL,e.to_string()))?;
 window.set_theme(naui_core::Theme::Dark).map_err(|e|windows_core::Error::new(windows::Win32::Foundation::E_FAIL,e.to_string()))?;
 let glass = crate::presentation::apply_glass(&window);
 match glass {
     Ok(class) => log(data, &format!("BACKDROP {class}")),
     Err(error) => log(data, &format!("BACKDROP fallback {error}")),
 }
 let root:FrameworkElement=XamlReader::Load(&HSTRING::from(include_str!("main.xaml")))?.cast()?;
 window.set_child(&Native(root.cast()?));
 let view=Rc::new(RefCell::new(View{rows:element(&root,"Rows")?,root:root.clone(),registry:HashMap::new(),order:vec![],checked:Rc::new(RefCell::new(BTreeSet::new())),tx:backend.tx.clone(),snapshot:Snapshot::default(),query:String::new(),sort:0,monitor_ids:vec![]}));
 for (name,kind) in [("Lock",0),("Unlock",1),("Restore",2),("Cancel",3)] {let v=view.clone();click(&root,name,move||{let v=v.borrow();let ids=v.selected();let cmd=match kind{0=>Command::Lock(ids),1=>Command::Unlock(ids),2=>Command::Restore(ids),_=>Command::CancelPending(ids)};let _=v.tx.send(cmd);})?;}
 for (name,cmd) in [("Refresh",Command::Refresh),("UnlockAll",Command::UnlockAll),("MouseRelease",Command::MouseLock(None))] {let tx=backend.tx.clone();let cmd=Rc::new(RefCell::new(Some(cmd)));click(&root,name,move||{let _= &cmd;let c=match name{"Refresh"=>Command::Refresh,"UnlockAll"=>Command::UnlockAll,_=>Command::MouseLock(None)};let _=tx.send(c);})?;}
 {let v=view.clone();click(&root,"Move",move||{let v=v.borrow();if let Ok(c)=element::<ComboBox>(&v.root,"Target"){if let Ok(i)=c.SelectedIndex(){if let Some(id)=v.monitor_ids.get(i as usize){let _=v.tx.send(Command::Move(v.selected(),id.clone()));}}}})?;}
 {let v=view.clone();click(&root,"MouseApply",move||{let v=v.borrow();if let Ok(c)=element::<ComboBox>(&v.root,"MouseTarget"){if let Ok(i)=c.SelectedIndex(){if let Some(id)=v.monitor_ids.get(i as usize){let _=v.tx.send(Command::MouseLock(Some(id.clone())));}}}})?;}
 for (name,add) in [("BlacklistAdd",true),("BlacklistRemove",false)] {let v=view.clone();click(&root,name,move||{let v=v.borrow();if let Ok(t)=element::<TextBox>(&v.root,"Blacklist"){if let Ok(s)=t.Text(){let s=s.to_string();if !s.trim().is_empty(){let _=v.tx.send(if add{Command::AddBlacklist(s)}else{Command::RemoveBlacklist(s)});}}}})?;}
 for (name,all) in [("SelectAll",true),("Clear",false)] {let v=view.clone();click(&root,name,move||{let mut v=v.borrow_mut();v.checked.borrow_mut().clear();if all{for r in &v.snapshot.windows{if !r.blacklisted&&(v.query.is_empty()||r.title.to_lowercase().contains(&v.query)||r.process.to_lowercase().contains(&v.query)){v.checked.borrow_mut().insert(r.id);}}}let s=v.snapshot.clone();let _=v.refresh(s);})?;}
 {let v=view.clone();let search:TextBox=element(&root,"Search")?;let text=search.clone();search.TextChanged(&text_changed(move ||{let mut v=v.borrow_mut();v.query=text.Text()?.to_string().to_lowercase();let s=v.snapshot.clone();v.refresh(s)?;Ok(())}))?;}
 {let v=view.clone();let sort:ComboBox=element(&root,"Sort")?;let combo=sort.clone();sort.SelectionChanged(&selection_changed(move ||{let mut v=v.borrow_mut();v.sort=combo.SelectedIndex()?;v.reset_order();let s=v.snapshot.clone();v.refresh(s)?;Ok(())}))?;}
 {let check:CheckBox=element(&root,"Autostart")?;let c=check.clone();let tx=backend.tx.clone();check.Click(&routed(move ||{let enabled=c.IsChecked().and_then(|b|b.Value()).unwrap_or(false);let _=tx.send(Command::SetAutostart(enabled));Ok(())}))?;}
 let mut placement = crate::presentation::Placement::new(&window, readonly)?;
 crate::presentation::apply_icon(placement.hwnd)?;
 window.show();
 placement.fit(true)?;
 log(data, "ICON resource=101");
 log(data,&format!("UI_READY class={}",window.native_window().cast::<IInspectable>()?.GetRuntimeClassName()?));
 for name in ["Microsoft.UI.Xaml.dll","Microsoft.WindowsAppRuntime.dll"] {unsafe {use windows::Win32::System::LibraryLoader::{GetModuleHandleW,GetModuleFileNameW};let wide=name.encode_utf16().chain(Some(0)).collect::<Vec<_>>();if let Ok(module)=GetModuleHandleW(windows_core::PCWSTR(wide.as_ptr())){let mut path=[0u16;4096];let n=GetModuleFileNameW(Some(module),&mut path);log(data,&format!("MODULE {name}={}",String::from_utf16_lossy(&path[..n as usize])));}}}
 let stopped=Arc::new(AtomicBool::new(false));let stop=stopped.clone();let tx=backend.tx.clone();let close_window=window.clone();window.on_close_request(move||{if smoke{stop.store(true,Ordering::Relaxed);let _=tx.send(Command::Shutdown);naui_core::CloseResponse::Close}else{if let Ok(native)=close_window.native_window().cast::<naui_winui3::IWindowNative>(){unsafe{if let Ok(hwnd)=native.WindowHandle(){let _=windows::Win32::UI::WindowsAndMessaging::ShowWindow(hwnd,windows::Win32::UI::WindowsAndMessaging::SW_HIDE);}}}naui_core::CloseResponse::KeepOpen}});
 let ticks=Rc::new(Cell::new(0u32));let tick_count=ticks.clone();let u=ui.clone();let win=window.clone();let path=PathBuf::from(data);let snapshot=backend.snapshot.clone();let stop_ui=stopped.clone();let tray=if smoke{None}else{Some(crate::tray::start(Some(placement.hwnd)))};
 let channel = ui.tasks().channel(move |()| {
     let snapshot = snapshot.lock().unwrap_or_else(|e| e.into_inner()).clone();
     if let Err(error) = view.borrow_mut().refresh(snapshot) {
         log(&path, &format!("UI_REFRESH_ERROR {error}"));
     }
     if let Err(error) = placement.update(false) {
         log(&path, &format!("PLACEMENT_ERROR {error}"));
     }
     if let Some(tray) = &tray {
         while let Ok(action) = tray.rx.try_recv() {
             match action {
                 crate::tray::TrayAction::Open => {
                     if unsafe { windows::Win32::UI::WindowsAndMessaging::IsIconic(placement.hwnd) }.as_bool() {
                         win.set_state(naui_core::WindowState::Normal);
                     }
                     win.show();
                     if let Err(error) = placement.update(true) {
                         log(&path, &format!("PLACEMENT_ERROR {error}"));
                     }
                     log(&path, "TRAY_OPEN");
                 }
                 crate::tray::TrayAction::MoveTo(device) => {
                     let display = window_lock_master_rust::placement::monitor_name(&device);
                     match placement.move_to(&win, device) {
                         Ok(()) => log(&path, &format!("TRAY_MOVE {display}")),
                         Err(error) => {
                             let message = format!("主窗口移动失败（{display}）\n{error}");
                             log(&path, &format!("TRAY_MOVE_ERROR {message}"));
                             use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONWARNING, MB_OK};
                             let text = HSTRING::from(message);
                             unsafe { MessageBoxW(None, &text, windows_core::w!("WindowLockMaster"), MB_OK | MB_ICONWARNING); }
                         }
                     }
                 }
                 crate::tray::TrayAction::Exit => {
                     log(&path, "TRAY_EXIT");
                     stop_ui.store(true, Ordering::Relaxed);
                     u.quit();
                 }
             }
         }
     }
     tick_count.set(tick_count.get() + 1);
     if smoke && tick_count.get() >= 15 {
         log(&path, "UI_SMOKE_OK");
         stop_ui.store(true, Ordering::Relaxed);
         u.quit();
     }
 });
 std::thread::spawn(move||{while !stopped.load(Ordering::Relaxed){std::thread::sleep(Duration::from_millis(200));if channel.send(()).is_err(){break;}}});
 Ok(())
}
