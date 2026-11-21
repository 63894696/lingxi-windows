import sqlite3
db=r'D:\Projects\lingxi-windows\installer\stage\models\ciku.db'
c=sqlite3.connect(db)
# ensure columns
cols=[r[1] for r in c.execute('pragma table_info(phrase)')]
print('cols', cols)
# insert rescue term if missing
exists=c.execute("select count(*) from phrase where key=? and value=?", ('niaoduzhengwanqi','尿毒症晚期')).fetchone()[0]
if not exists:
    if 'jp' in cols:
        c.execute("insert into phrase(key,jp,value,weight) values(?,?,?,?)", ('niaoduzhengwanqi','ndzwq','尿毒症晚期',20000))
    else:
        c.execute("insert into phrase(key,value,weight) values(?,?,?)", ('niaoduzhengwanqi','尿毒症晚期',20000))
    c.commit()
    print('inserted')
else:
    print('already exists')
print(c.execute("select key,value,weight from phrase where key='niaoduzhengwanqi'").fetchall())
