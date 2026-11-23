// QEDIT-B: draggable, DataTransfer (data store only), DataTransferItemList, DataTransferItem, FileList, DragEvent.
#[allow(unused_imports)]
use super::*;

#[test]
fn qedit_b_draggable_reflection() {
    let l = run_in_html(
        "<html><body><a id=a href='#'>x</a><a id=n>x</a><div id=d></div><img id=i><img id=f draggable=FaLsE><img id=k draggable='falſe'>\
         <div id=t draggable=true></div></body></html>",
        "var $ = function (x) { return document.getElementById(x); };\
         console.log($('a').draggable, $('n').draggable, $('d').draggable, $('i').draggable, $('f').draggable, $('k').draggable, $('t').draggable);\
         $('d').draggable = true; console.log($('d').getAttribute('draggable'));\
         $('d').draggable = 'foo'; console.log($('d').draggable);\
         $('i').draggable = false; console.log($('i').getAttribute('draggable'), $('i').draggable);\
         $('t').setAttribute('draggable', 'auto'); console.log($('t').draggable);",
    );
    assert_eq!(l[0], "log:true false false true false true true", "{l:?}");
    assert_eq!(l[1], "log:true");
    assert_eq!(l[2], "log:true");
    assert_eq!(l[3], "log:false false");
    assert_eq!(l[4], "log:false");
}

#[test]
fn qedit_b_datatransfer_store() {
    let l = run_in_html(
        "<html><body></body></html>",
        "var dt = new DataTransfer();\
         console.log(dt.dropEffect, dt.effectAllowed, dt.items.length, dt.types.length, dt.files.length);\
         dt.dropEffect = 'bogus'; dt.effectAllowed = 'copyMove'; dt.dropEffect = 'link'; dt.effectAllowed = 'nope';\
         console.log(dt.dropEffect, dt.effectAllowed);\
         dt.setData('text', 'hi'); dt.setData('URL', '# c\\r\\nhttp://a/1\\nhttp://b/2');\
         console.log(dt.getData('text/plain'), dt.getData('Text'), dt.getData(' url '), dt.types.join());\
         var t = dt.types; console.log(t === dt.types, Object.isFrozen(t));\
         dt.items.add(new File(['abc'], 'a.txt', { type: 'text/plain' }));\
         console.log(dt.types.join(), dt.files.length, dt.files[0].name, dt.files.item(0) === dt.files[0], dt.items.length);\
         dt.clearData(); console.log(dt.types.join(), dt.items.length);\
         try { dt.items.add('x', 'text/plain'); dt.items.add('y', 'TEXT/PLAIN'); } catch (e) { console.log(e.name); }\
         var it = dt.items[1]; console.log(it.kind, it.type, it.getAsFile());\
         dt.items.remove(1); console.log(it.kind === '', dt.items.length, dt.items[1]);\
         dt.items.clear(); console.log(dt.items.length, dt.files.length);\
         console.log(Object.prototype.toString.call(dt), Object.prototype.toString.call(dt.files), Object.keys(dt.items).length);",
    );
    assert_eq!(l[0], "log:none none 0 0 0", "{l:?}");
    assert_eq!(l[1], "log:link copyMove");
    assert_eq!(l[2], "log:hi hi http://a/1 text/plain,text/uri-list");
    assert_eq!(l[3], "log:true true");
    assert_eq!(l[4], "log:text/plain,text/uri-list,Files 1 a.txt true 3");
    assert_eq!(l[5], "log:Files 1");
    assert_eq!(l[6], "log:NotSupportedError");
    assert_eq!(l[7], "log:string text/plain null");
    assert_eq!(l[8], "log:true 1 undefined");
    assert_eq!(l[9], "log:0 0");
}

#[test]
fn qedit_b_drag_event_data_transfer() {
    let l = run_in_html(
        "<html><body></body></html>",
        "var dt = new DataTransfer();\
         var e = new DragEvent('drop', { dataTransfer: dt }); console.log(e.dataTransfer === dt, new DragEvent('drop').dataTransfer);\
         try { new DragEvent('drop', { dataTransfer: {} }); } catch (x) { console.log(x.name); }\
         try { new DataTransferItem(); } catch (x) { console.log(x.name); }\
         try { new FileList(); } catch (x) { console.log(x.name); }\
         var p = __dt_make('readonly'); p.setData('text/plain', 'x'); console.log(p.items.length, p.items.add('a', 'b'));\
         try { p.items.remove(0); } catch (x) { console.log(x.name); }",
    );
    assert_eq!(l[0], "log:true null", "{l:?}");
    assert_eq!(l[1], "log:TypeError");
    assert_eq!(l[2], "log:TypeError");
    assert_eq!(l[3], "log:TypeError");
    assert_eq!(l[4], "log:0 null");
    assert_eq!(l[5], "log:InvalidStateError");
}
