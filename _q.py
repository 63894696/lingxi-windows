import sqlite3
c=sqlite3.connect(r'D:\Projects\lingxi-windows\installer\stage\models\ciku.db')
print('like', c.execute("select key,value,weight from phrase where key like 'niaodu%' or value like '%尿毒症%' limit 20").fetchall())
print('exact', c.execute("select key,value,weight from phrase where key='niaoduzhengwanqi'").fetchall())
