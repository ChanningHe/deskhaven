# First steps

When you finish, you have a second user, two-factor authentication on your account, and a
strategy that sets client options on every device.

## Create a user

1. Open **Users** and choose **New user**.
2. Enter a username and choose **Create user**.
3. Give the user the temporary password shown. They choose their own at their first
   sign-in to the console.

## Turn on two-factor authentication

1. Open **My account** and choose **Enable two-factor authentication**.
2. Scan the QR code with an authenticator app, then enter your password and the code.
3. Store the recovery codes somewhere safe
   ([where they work](../reference/known-limitations.md#accounts-and-sign-in)).

To require it for others, open **Identity** and set **Require two-factor authentication
for**. A user it covers enrols in the console before the RustDesk client lets them sign in.

## Create a strategy

1. Open **Strategies** and choose **New strategy**. Name it and create it.
2. On **Options**, under **Permissions**, set **File transfer** to **Off** and choose
   **Save**.
3. On **Assignments**, leave **Target type** at **Global**, choose **Assign**, then
   **Assign globally**.

Every device applies the strategy on its next heartbeat, about 15 seconds later.

More: [guides](../README.md#guides).
