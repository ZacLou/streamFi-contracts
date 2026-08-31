import { 
  TransactionBuilder, 
  Contract, 
  Address, 
  xdr,
  Server as SorobanRpc,
  Networks 
} from '@stellar/stellar-sdk';

export interface StreamConfig {
  sender: string;
  recipient: string;
  token: string;
  ratePerSecond: string;
  startTime: string;
  endTime: string;
  clawbackEnabled: boolean;
}

export interface StreamInfo {
  sender: string;
  recipient: string;
  token: string;
  rate_per_second: string;
  start_time: string;
  end_time: string;
  withdrawn: string;
  paused_at: string;
  flags: string;
}

export class StreamSDK {
  private server: SorobanRpc;
  private networkPassphrase: string;

  constructor(rpcUrl: string, networkPassphrase?: string) {
    this.server = new SorobanRpc(rpcUrl);
    this.networkPassphrase = networkPassphrase || 
      (rpcUrl.includes('mainnet') 
        ? Networks.PUBLIC 
        : Networks.TESTNET);
  }

  /**
   * Set an operator for a stream
   * Only the sender can call this
   */
  async setOperator(
    streamId: string,
    caller: string,
    operator: string,
    sourceAccount: string
  ): Promise<string> {
    const account = await this.server.getAccount(sourceAccount);
    
    const transaction = new TransactionBuilder(account, {
      fee: '100',
      networkPassphrase: this.networkPassphrase,
    })
      .addOperation(
        Contract.invokeFunction({
          contract: streamId,
          function: 'set_operator',
          args: [
            xdr.ScVal.scvAddress(Address.fromString(caller)),
            xdr.ScVal.scvAddress(Address.fromString(operator)),
          ],
        })
      )
      .setTimeout(30)
      .build();

    const prepared = await this.server.prepareTransaction(transaction);
    return prepared.toXDR();
  }

  /**
   * Revoke the operator for a stream
   * Only the sender can call this
   */
  async revokeOperator(
    streamId: string,
    caller: string,
    sourceAccount: string
  ): Promise<string> {
    const account = await this.server.getAccount(sourceAccount);
    
    const transaction = new TransactionBuilder(account, {
      fee: '100',
      networkPassphrase: this.networkPassphrase,
    })
      .addOperation(
        Contract.invokeFunction({
          contract: streamId,
          function: 'revoke_operator',
          args: [
            xdr.ScVal.scvAddress(Address.fromString(caller)),
          ],
        })
      )
      .setTimeout(30)
      .build();

    const prepared = await this.server.prepareTransaction(transaction);
    return prepared.toXDR();
  }

  /**
   * Extend the duration of a stream
   * Only sender or operator can call this
   */
  async extendDuration(
    streamId: string,
    caller: string,
    extraTimeSeconds: string,
    sourceAccount: string
  ): Promise<string> {
    const account = await this.server.getAccount(sourceAccount);
    
    const transaction = new TransactionBuilder(account, {
      fee: '100',
      networkPassphrase: this.networkPassphrase,
    })
      .addOperation(
        Contract.invokeFunction({
          contract: streamId,
          function: 'extend_duration',
          args: [
            xdr.ScVal.scvAddress(Address.fromString(caller)),
            xdr.ScVal.scvU64(xdr.Uint64.fromString(extraTimeSeconds)),
          ],
        })
      )
      .setTimeout(30)
      .build();

    const prepared = await this.server.prepareTransaction(transaction);
    return prepared.toXDR();
  }

  /**
   * Top up and extend duration in a single operation
   * Only sender or operator can call this
   */
  async topUpAndExtend(
    streamId: string,
    caller: string,
    amount: string,
    extraTimeSeconds: string,
    sourceAccount: string
  ): Promise<string> {
    const account = await this.server.getAccount(sourceAccount);
    
    const transaction = new TransactionBuilder(account, {
      fee: '100',
      networkPassphrase: this.networkPassphrase,
    })
      .addOperation(
        Contract.invokeFunction({
          contract: streamId,
          function: 'top_up_and_extend',
          args: [
            xdr.ScVal.scvAddress(Address.fromString(caller)),
            xdr.ScVal.scvI128(xdr.Int128.fromString(amount)),
            xdr.ScVal.scvU64(xdr.Uint64.fromString(extraTimeSeconds)),
          ],
        })
      )
      .setTimeout(30)
      .build();

    const prepared = await this.server.prepareTransaction(transaction);
    return prepared.toXDR();
  }

  /**
   * Get the current operator for a stream
   */
  async getOperator(streamId: string): Promise<string | null> {
    const result = await this.server.getContractData(
      streamId,
      xdr.ScVal.scvSymbol('operator')
    );
    
    if (result && result.val().address) {
      return result.val().address().toString();
    }
    return null;
  }

  /**
   * Get stream information
   */
  async getStreamInfo(streamId: string): Promise<StreamInfo> {
    const result = await this.server.getContractData(
      streamId,
      xdr.ScVal.scvSymbol('info')
    );
    
    if (!result) {
      throw new Error('Stream info not found');
    }

    const val = result.val();
    return {
      sender: val.vec()?.[0]?.address()?.toString() || '',
      recipient: val.vec()?.[1]?.address()?.toString() || '',
      token: val.vec()?.[2]?.address()?.toString() || '',
      rate_per_second: val.vec()?.[3]?.i128()?.toString() || '0',
      start_time: val.vec()?.[4]?.u64()?.toString() || '0',
      end_time: val.vec()?.[5]?.u64()?.toString() || '0',
      withdrawn: val.vec()?.[6]?.i128()?.toString() || '0',
      paused_at: val.vec()?.[7]?.u64()?.toString() || '0',
      flags: val.vec()?.[8]?.u32()?.toString() || '0',
    };
  }

  /**
   * Get withdrawable balance for a stream
   */
  async getWithdrawable(streamId: string): Promise<string> {
    const result = await this.server.getContractData(
      streamId,
      xdr.ScVal.scvSymbol('withdrawable')
    );
    
    if (result && result.val().i128) {
      return result.val().i128().toString();
    }
    return '0';
  }

  /**
   * Check if clawback is enabled for a stream
   */
  async isClawbackEnabled(streamId: string): Promise<boolean> {
    const result = await this.server.getContractData(
      streamId,
      xdr.ScVal.scvSymbol('clawback_enabled')
    );
    
    if (result && result.val().bool !== undefined) {
      return result.val().bool();
    }
    return false;
  }

  /**
   * Get total streamed amount
   */
  async getStreamedTotal(streamId: string): Promise<string> {
    const result = await this.server.getContractData(
      streamId,
      xdr.ScVal.scvSymbol('total_streamed')
    );
    
    if (result && result.val().i128) {
      return result.val().i128().toString();
    }
    return '0';
  }

  /** Alias matching the on-chain `total_streamed()` view method. */
  async getTotalStreamed(streamId: string): Promise<string> {
    return this.getStreamedTotal(streamId);
  }
}