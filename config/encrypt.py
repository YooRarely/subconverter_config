import binascii
import os
import sys

def xor_process_file(input_file, output_file, key):
    try:
        # 1. 读取原始文件内容
        with open(input_file, 'r', encoding='utf-8') as f:
            data = f.read().strip()
        
        data_bytes = data.encode('utf-8')
        key_bytes = key.encode('utf-8')
        key_len = len(key_bytes)
        
        # 2. 异或运算
        encrypted_bytes = bytearray(
            data_bytes[i] ^ key_bytes[i % key_len] 
            for i in range(len(data_bytes))
        )
        
        # 3. 转换为十六进制并写入新文件
        hex_string = binascii.hexlify(encrypted_bytes)
        with open(output_file, 'wb') as f:
            f.write(hex_string)
            
        print(f"✅ 成功！已读取 '{input_file}'，加密结果已保存至 '{output_file}'")
        
    except FileNotFoundError:
        print(f"❌ 错误：找不到文件 '{input_file}'")
    except Exception as e:
        print(f"❌ 发生未知错误: {e}")

if __name__ == "__main__":
    # 从环境变量读取 Key
    KEY = os.environ.get("XOR_KEY")
    
    if not KEY:
        print("❌ 错误: 请先设置环境变量 XOR_KEY")
        sys.exit(1)

    # 执行转换：读取 users.json，写入 users.db
    xor_process_file("subscribe.json", "man_what_can_i_say", KEY)
