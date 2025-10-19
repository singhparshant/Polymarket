from py_clob_client.client import ClobClient
from py_clob_client.clob_types import OpenOrderParams

# from py_clob_client.clob_types import OpenOrderParams
import os
import dotenv

dotenv.load_dotenv()

print(os.getenv("PK"))

client = ClobClient(
    host="https://clob.polymarket.com",
    key=os.getenv("PK"),
    chain_id=137,
)

client.set_api_creds(client.create_or_derive_api_creds())

# resp = client.get_trades()

address = client.get_orders(
    # OpenOrderParams(
    #     market="0x91dbde104fee004a34f638f9a49d7fb8d4e8bc75e213aab92cd853cbbc4c9c90",
    # )
    # None
)
print(address)
# print(resp)
print("Done!")
